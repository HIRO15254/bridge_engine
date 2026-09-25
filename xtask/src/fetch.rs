//! `corpus fetch` and `systems fetch`: download every `[[entry]]` of a manifest, verify its
//! SHA-256, and unpack archives.
//!
//! Manifest format (`corpus/manifest.toml`, `systems/vendor/manifest.toml`):
//!
//! ```toml
//! [[entry]]
//! name = "dds-list100"                 # unique; used by --only and by --pin
//! description = "..."                  # free text (ignored here)
//! url = "https://..."
//! sha256 = ""                          # "" = not pinned yet: fetch, print the hash, --pin records it
//! unpack = "none"                      # "none" (dest is the file) | "zip" (dest is a directory)
//! dest = "dds/list100.txt"             # relative to the data directory
//! formats = ["deal-string"]            # for the tests (ignored here)
//! note = "..."                         # optional, free text (ignored here)
//! ```
//!
//! For `unpack = "zip"` the archive is kept next to the extracted directory as `<dest>.zip` so
//! that a later run can verify it without downloading again. Data that does not match a pinned
//! hash is deleted and never left on disk.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::Result;

const USER_AGENT: &str = "bridge_engine-xtask/0.0.1 (+https://github.com/HIRO15254/bridge_engine)";
/// Minimum interval between two requests to the same host.
const HOST_PAUSE: Duration = Duration::from_secs(1);

/// Which manifest / data directory pair a `fetch` command works on.
#[derive(Clone, Copy)]
pub enum Target {
    /// `corpus/manifest.toml` -> `corpus/data/` (or `$BRIDGE_CORPUS_DIR`).
    Corpus,
    /// `systems/vendor/manifest.toml` -> `systems/vendor/data/` (or `$BRIDGE_SYSTEMS_DIR/vendor/data/`).
    Systems,
}

impl Target {
    fn manifest(self, root: &Path) -> PathBuf {
        match self {
            Target::Corpus => root.join("corpus/manifest.toml"),
            Target::Systems => root.join("systems/vendor/manifest.toml"),
        }
    }

    fn data_dir(self, root: &Path) -> PathBuf {
        match self {
            Target::Corpus => std::env::var_os("BRIDGE_CORPUS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join("corpus/data")),
            Target::Systems => std::env::var_os("BRIDGE_SYSTEMS_DIR")
                .map(|dir| PathBuf::from(dir).join("vendor/data"))
                .unwrap_or_else(|| root.join("systems/vendor/data")),
        }
    }
}

#[derive(Deserialize)]
struct Manifest {
    entry: Vec<Entry>,
}

/// One `[[entry]]`. Fields the tool does not need (`description`, `formats`, `license`, `note`)
/// are not declared and are ignored by serde.
#[derive(Deserialize)]
struct Entry {
    name: String,
    url: String,
    sha256: String,
    #[serde(default)]
    unpack: Unpack,
    dest: String,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
enum Unpack {
    #[default]
    None,
    Zip,
}

struct Options {
    pin: bool,
    force: bool,
    only: Vec<String>,
}

fn parse_options(args: &[&str]) -> Result<Options> {
    let mut opts = Options {
        pin: false,
        force: false,
        only: Vec::new(),
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match *arg {
            "--pin" => opts.pin = true,
            "--force" => opts.force = true,
            "--only" => match iter.next() {
                Some(name) => opts.only.push((*name).to_owned()),
                None => return Err("--only needs an entry name".into()),
            },
            other => return Err(format!("unknown option `{other}`\n{}", crate::USAGE).into()),
        }
    }
    Ok(opts)
}

enum Status {
    /// Downloaded and the hash matched the manifest.
    Verified,
    /// Already on disk with the pinned hash; nothing downloaded.
    Cached,
    /// Was unpinned; downloaded and the hash was written into the manifest (`--pin`).
    Pinned,
    /// Was unpinned; downloaded and the hash printed, manifest untouched.
    Unpinned,
    Failed(String),
}

struct Row {
    name: String,
    status: Status,
    bytes: u64,
    sha256: String,
}

/// Entry point of `corpus fetch` / `systems fetch`.
pub fn run(target: Target, args: &[&str]) -> Result<ExitCode> {
    let opts = parse_options(args)?;
    let root = crate::workspace_root();
    let manifest_path = target.manifest(&root);
    let data_dir = target.data_dir(&root);

    let text = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest: Manifest =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    for name in &opts.only {
        if !manifest.entry.iter().any(|e| &e.name == name) {
            return Err(format!("no entry named `{name}` in {}", manifest_path.display()).into());
        }
    }
    let entries: Vec<&Entry> = manifest
        .entry
        .iter()
        .filter(|e| opts.only.is_empty() || opts.only.contains(&e.name))
        .collect();

    fs::create_dir_all(&data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?;
    eprintln!(
        "fetching {} entries of {} into {}",
        entries.len(),
        manifest_path.display(),
        data_dir.display()
    );

    let mut fetcher = Fetcher::new();
    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        eprintln!("[{}]", entry.name);
        let row = match fetch_entry(&mut fetcher, entry, &data_dir, &opts, &manifest_path) {
            Ok((status, bytes, sha256)) => Row {
                name: entry.name.clone(),
                status,
                bytes,
                sha256,
            },
            Err(err) => {
                eprintln!("  FAILED: {err}");
                Row {
                    name: entry.name.clone(),
                    status: Status::Failed(err.to_string()),
                    bytes: 0,
                    sha256: String::new(),
                }
            }
        };
        rows.push(row);
    }

    print_table(&rows);
    let failed = rows
        .iter()
        .filter(|r| matches!(r.status, Status::Failed(_)))
        .count();
    let unpinned = rows
        .iter()
        .filter(|r| matches!(r.status, Status::Unpinned))
        .count();
    Ok(if failed > 0 {
        eprintln!("{failed} entries failed");
        ExitCode::from(1)
    } else if unpinned > 0 {
        eprintln!("{unpinned} entries are not pinned; paste the hashes or rerun with --pin");
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    })
}

/// Fetches (or verifies the cached copy of) one entry and returns its status, size and hash.
fn fetch_entry(
    fetcher: &mut Fetcher,
    entry: &Entry,
    data_dir: &Path,
    opts: &Options,
    manifest_path: &Path,
) -> Result<(Status, u64, String)> {
    validate_dest(entry)?;
    let dest = data_dir.join(&entry.dest);
    let file = match entry.unpack {
        Unpack::None => dest.clone(),
        Unpack::Zip => data_dir.join(format!("{}.zip", entry.dest.trim_end_matches('/'))),
    };
    let pinned = !entry.sha256.is_empty();

    if pinned && !opts.force && file.is_file() {
        let (sha256, bytes) = sha256_file(&file)?;
        if sha256 == entry.sha256 {
            if entry.unpack == Unpack::Zip && !dest.is_dir() {
                let n = unzip(&file, &dest)?;
                eprintln!("  extracted {n} files into {}", dest.display());
            }
            eprintln!("  verified (cached): {} bytes", bytes);
            return Ok((Status::Cached, bytes, sha256));
        }
        eprintln!("  cached file has a different hash; downloading again");
    }

    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = PathBuf::from(format!("{}.part", file.display()));
    let (sha256, bytes) = fetcher.download(&entry.url, &tmp)?;
    eprintln!("  {bytes} bytes, sha256 = \"{sha256}\"");

    if pinned && sha256 != entry.sha256 {
        let _ = fs::remove_file(&tmp);
        return Err(format!(
            "sha256 mismatch: manifest has {}, downloaded {sha256} ({bytes} bytes); file deleted",
            entry.sha256
        )
        .into());
    }
    if entry.unpack == Unpack::None && looks_like_html(&tmp)? && !is_html_dest(&entry.dest) {
        let _ = fs::remove_file(&tmp);
        return Err("the response is an HTML page, not the expected data; file deleted".into());
    }
    fs::rename(&tmp, &file)?;
    if entry.unpack == Unpack::Zip {
        let n = unzip(&file, &dest)?;
        eprintln!("  extracted {n} files into {}", dest.display());
    }

    if pinned {
        Ok((Status::Verified, bytes, sha256))
    } else if opts.pin {
        pin_in_manifest(manifest_path, &entry.name, &sha256)?;
        eprintln!("  pinned in {}", manifest_path.display());
        Ok((Status::Pinned, bytes, sha256))
    } else {
        eprintln!("  not pinned: paste the hash into the manifest or rerun with --pin");
        Ok((Status::Unpinned, bytes, sha256))
    }
}

/// `dest` must stay inside the data directory and, for archives, must not be the data
/// directory itself (it is deleted before extraction).
fn validate_dest(entry: &Entry) -> Result<()> {
    let dest = Path::new(&entry.dest);
    let bad = entry.dest.is_empty()
        || dest.is_absolute()
        || dest.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::CurDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        });
    if bad {
        return Err(format!(
            "entry `{}`: dest must be a relative path without `..`",
            entry.name
        )
        .into());
    }
    if entry.unpack == Unpack::None && entry.dest.ends_with('/') {
        return Err(format!(
            "entry `{}`: dest of a plain file must not end with `/`",
            entry.name
        )
        .into());
    }
    Ok(())
}

fn is_html_dest(dest: &str) -> bool {
    let lower = dest.to_ascii_lowercase();
    lower.ends_with(".html") || lower.ends_with(".htm")
}

/// True when the file starts like an HTML document (error pages served with status 200).
fn looks_like_html(path: &Path) -> Result<bool> {
    let mut head = [0u8; 512];
    let n = File::open(path)?.read(&mut head)?;
    let text = String::from_utf8_lossy(&head[..n]).to_ascii_lowercase();
    let text = text.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    Ok(text.starts_with("<!doctype html") || text.starts_with("<html"))
}

/// Downloads with one HTTP client, one request at a time, pausing between requests to the
/// same host.
pub struct Fetcher {
    agent: ureq::Agent,
    last_request: HashMap<String, Instant>,
}

impl Fetcher {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .max_redirects(10)
            .http_status_as_error(true)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_global(Some(Duration::from_secs(15 * 60)))
            .build();
        Fetcher {
            agent: ureq::Agent::new_with_config(config),
            last_request: HashMap::new(),
        }
    }

    /// Streams `url` into `tmp` and returns `(sha256, bytes)`. `tmp` is removed on error.
    pub fn download(&mut self, url: &str, tmp: &Path) -> Result<(String, u64)> {
        self.pause_for_host(url);
        eprintln!("  GET {url}");
        let result = self.stream(url, tmp);
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result.map_err(|e| format!("{url}: {e}").into())
    }

    fn stream(&self, url: &str, tmp: &Path) -> Result<(String, u64)> {
        let mut response = self
            .agent
            .get(url)
            .header("User-Agent", USER_AGENT)
            .call()?;
        let mut reader = response.body_mut().as_reader();
        let mut out = File::create(tmp)?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        let mut bytes = 0u64;
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n])?;
            bytes += n as u64;
        }
        out.flush()?;
        Ok((hex(&hasher.finalize()), bytes))
    }

    fn pause_for_host(&mut self, url: &str) {
        let host = host_of(url).to_owned();
        if let Some(last) = self.last_request.get(&host) {
            let elapsed = last.elapsed();
            if elapsed < HOST_PAUSE {
                std::thread::sleep(HOST_PAUSE - elapsed);
            }
        }
        self.last_request.insert(host, Instant::now());
    }
}

fn host_of(url: &str) -> &str {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    after_scheme.split(['/', '?', '#']).next().unwrap_or("")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 and size of a file on disk.
pub fn sha256_file(path: &Path) -> Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        bytes += n as u64;
    }
    Ok((hex(&hasher.finalize()), bytes))
}

/// Extracts `archive` into `dest` (which is recreated) and returns the number of files.
/// Entries whose name would escape `dest` abort the extraction.
fn unzip(archive: &Path, dest: &Path) -> Result<usize> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    let mut count = 0;
    for index in 0..zip.len() {
        let mut file = zip.by_index(index)?;
        let Some(relative) = file.enclosed_name() else {
            return Err(format!(
                "{}: refusing to extract unsafe path {:?}",
                archive.display(),
                file.name()
            )
            .into());
        };
        let target = dest.join(relative);
        if file.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        io::copy(&mut file, &mut File::create(&target)?)?;
        count += 1;
    }
    Ok(count)
}

/// Replaces the `sha256 = ""` line of the `[[entry]]` named `name`, leaving every other byte
/// of the manifest (comments, formatting) untouched.
fn pin_in_manifest(path: &Path, name: &str, sha256: &str) -> Result<()> {
    let text = fs::read_to_string(path)?;
    let mut out = String::with_capacity(text.len() + 64);
    let mut in_entry = false;
    let mut replaced = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("[[") {
            in_entry = false;
        } else if let Some(value) = key_value(trimmed, "name") {
            in_entry = value == name;
        }
        if in_entry && !replaced && key_value(trimmed, "sha256") == Some("") {
            let indent = &line[..line.len() - line.trim_start().len()];
            let newline = if line.ends_with('\n') { "\n" } else { "" };
            out.push_str(&format!("{indent}sha256 = \"{sha256}\"{newline}"));
            replaced = true;
        } else {
            out.push_str(line);
        }
    }
    if !replaced {
        return Err(format!(
            "could not find `sha256 = \"\"` for entry `{name}` in {}",
            path.display()
        )
        .into());
    }
    fs::write(path, out)?;
    Ok(())
}

/// For a line of the form `key = "value"` (optionally followed by a comment) returns `value`
/// when the key is exactly `key`.
fn key_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?.trim_start().strip_prefix('=')?;
    let rest = rest.trim_start().strip_prefix('"')?;
    rest.find('"').map(|end| &rest[..end])
}

fn print_table(rows: &[Row]) {
    let width = rows.iter().map(|r| r.name.len()).max().unwrap_or(4).max(4);
    println!();
    println!(
        "{:<width$}  {:<9}  {:>12}  sha256",
        "name", "status", "bytes"
    );
    for row in rows {
        let (status, detail) = match &row.status {
            Status::Verified => ("verified", row.sha256.as_str()),
            Status::Cached => ("cached", row.sha256.as_str()),
            Status::Pinned => ("pinned", row.sha256.as_str()),
            Status::Unpinned => ("UNPINNED", row.sha256.as_str()),
            Status::Failed(message) => ("FAILED", message.as_str()),
        };
        println!(
            "{:<width$}  {status:<9}  {:>12}  {detail}",
            row.name, row.bytes
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_value_parses_quoted_values() {
        assert_eq!(
            key_value(r#"name = "dds-list100""#, "name"),
            Some("dds-list100")
        );
        assert_eq!(key_value(r#"sha256 = """#, "sha256"), Some(""));
        assert_eq!(
            key_value(r#"sha256="abc"  # comment"#, "sha256"),
            Some("abc")
        );
        assert_eq!(key_value(r#"names = "x""#, "name"), None);
        assert_eq!(key_value(r#"formats = ["pbn"]"#, "formats"), None);
    }

    #[test]
    fn host_of_strips_scheme_path_and_query() {
        assert_eq!(
            host_of("https://www.bridgebase.com/tools/x.php?id=1"),
            "www.bridgebase.com"
        );
        assert_eq!(host_of("https://example.org"), "example.org");
        assert_eq!(host_of("example.org/a"), "example.org");
    }

    #[test]
    fn pin_rewrites_only_the_named_entry() {
        let manifest = "# sha256 = \"\" means unpinned\n\n[[entry]]\nname = \"a\"\nsha256 = \"\"\n\n[[entry]]\n  name = \"b\"\n  sha256 = \"\"  # keep\n";
        let path = std::env::temp_dir().join(format!("xtask-pin-test-{}.toml", std::process::id()));
        fs::write(&path, manifest).unwrap();
        pin_in_manifest(&path, "b", "cafe").unwrap();
        let out = fs::read_to_string(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(
            out,
            "# sha256 = \"\" means unpinned\n\n[[entry]]\nname = \"a\"\nsha256 = \"\"\n\n[[entry]]\n  name = \"b\"\n  sha256 = \"cafe\"\n"
        );
        let missing = pin_in_manifest(&path, "a", "x");
        assert!(missing.is_err());
    }

    #[test]
    fn unzip_refuses_path_traversal() {
        use zip::write::SimpleFileOptions;
        let mut cursor = io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            writer.start_file("../evil.txt", options).unwrap();
            writer.write_all(b"x").unwrap();
            writer.finish().unwrap();
        }
        let dir = std::env::temp_dir().join(format!("xtask-unzip-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("evil.zip");
        fs::write(&archive, cursor.into_inner()).unwrap();
        let result = unzip(&archive, &dir.join("out"));
        let escaped = dir.join("evil.txt").exists();
        fs::remove_dir_all(&dir).unwrap();
        assert!(result.is_err(), "traversal entry must be rejected");
        assert!(!escaped, "nothing may be written outside the destination");
    }

    #[test]
    fn dest_validation_rejects_escapes() {
        let entry = |dest: &str, unpack: Unpack| Entry {
            name: "t".into(),
            url: String::new(),
            sha256: String::new(),
            unpack,
            dest: dest.into(),
        };
        assert!(validate_dest(&entry("pbn/a.pbn", Unpack::None)).is_ok());
        assert!(validate_dest(&entry("pbn/a/", Unpack::Zip)).is_ok());
        assert!(validate_dest(&entry("../a.pbn", Unpack::None)).is_err());
        assert!(validate_dest(&entry("/tmp/a.pbn", Unpack::None)).is_err());
        assert!(validate_dest(&entry("", Unpack::Zip)).is_err());
        assert!(validate_dest(&entry("./", Unpack::Zip)).is_err());
        assert!(validate_dest(&entry("pbn/a/", Unpack::None)).is_err());
    }
}
