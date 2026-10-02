//! Compiles the vendored DDS 2.9.0 sources with `cc`.
//!
//! The sources are not committed; `cargo xtask dds vendor` downloads them into
//! `vendor/dds-2.9.0/`. When they are absent the crate still builds, with the FFI layer
//! compiled out (`cfg(dds_vendored)` unset) so that the workspace checks on every machine.
//!
//! Threading backend: the vendored sources pick their concurrency implementation from the
//! `DDS_THREADS_*` macros (`System.cpp`); with none defined DDS runs single-threaded. This crate
//! always defines `DDS_THREADS_STL` (`std::thread`, no extra library, works with MSVC/clang/gcc
//! alike). With the `openmp` feature it additionally defines `DDS_THREADS_OPENMP`, once a probe
//! has found a usable OpenMP runtime (CI only exercises this on Linux/gcc; see `VENDOR.md`).
//! `System::Reset` then takes the defined backend with the lowest index (`DDS_SYSTEM_THREAD_*`
//! in `System.cpp`: OpenMP is 2, STL is 5), so OpenMP wins and the STL code stays compiled in
//! but unused.
//!
//! Platform notes (see `docs/design/10-dds.md` D10 and R6/R14):
//! - The vendored `include/portab.h` (pulled in by every `.cpp` through `dds.h`) is what
//!   `#include <windows.h>` / `<unistd.h>` and picks the 64-bit `__int64` typedef per OS; it
//!   must be vendored alongside `include/dll.h` (`cargo xtask dds vendor`).
//! - MSVC does not accept `/std:c++11` (its minimum is `c++14`); gcc/clang accept `c++11`
//!   directly, which is what the vendored Makefiles use.
//! - Vendor warnings are silenced wholesale (`-w` / `-W0`) rather than patched: the sources are
//!   third-party and unmodified (see `VENDOR.md`).
use std::path::Path;

const SOURCES: &[&str] = &[
    "dds",
    "dump",
    "ABsearch",
    "ABstats",
    "CalcTables",
    "DealerPar",
    "File",
    "Init",
    "LaterTricks",
    "Memory",
    "Moves",
    "Par",
    "PlayAnalyser",
    "PBN",
    "QuickTricks",
    "Scheduler",
    "SolveBoard",
    "SolverIF",
    "System",
    "ThreadMgr",
    "Timer",
    "TimerGroup",
    "TimerList",
    "TimeStat",
    "TimeStatList",
    "TransTableS",
    "TransTableL",
];

fn main() {
    println!("cargo:rustc-check-cfg=cfg(dds_vendored)");
    println!("cargo:rerun-if-changed=vendor/dds-2.9.0");
    println!("cargo:rerun-if-changed=src/layout_probe.cpp");
    println!("cargo:rerun-if-changed=src/ffi_guard.cpp");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_OPENMP");

    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        return; // lib.rs emits a compile_error! for wasm32
    }

    let src = Path::new("vendor/dds-2.9.0/src");
    if !src.join("dds.cpp").exists() || !Path::new("vendor/dds-2.9.0/include/portab.h").exists() {
        println!(
            "cargo:warning=bridge-dds: DDS sources not vendored; run `cargo xtask dds vendor` (FFI disabled)"
        );
        return;
    }

    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let msvc = target_env == "msvc";

    let mut build = cc::Build::new();
    build
        .cpp(true)
        // MSVC's `/std` only goes down to c++14; gcc/clang accept c++11 as the vendored
        // Makefiles use. Either standard compiles these C++11-only sources unchanged.
        .std(if msvc { "c++14" } else { "c++11" })
        .opt_level(3)
        .include("vendor/dds-2.9.0/include")
        .include(src)
        .files(SOURCES.iter().map(|f| src.join(format!("{f}.cpp"))))
        .file("src/layout_probe.cpp")
        .file("src/ffi_guard.cpp")
        .define("DDS_THREADS_STL", None)
        // Third-party sources, unmodified: silence their warnings instead of patching them
        // (`-w` on gcc/clang, `/W0` on MSVC).
        .warnings(false)
        // Exercised on MSVC only (harmless elsewhere: `flag_if_supported` drops it when the
        // active compiler rejects it). `<stdexcept>` is pulled in by `Par.cpp`; MSVC needs an
        // explicit exception model or `std::vector`/`std::string` allocation failures are UB.
        .flag_if_supported("-EHsc");
    if msvc {
        // Silence CRT "insecure" warnings for `fopen`/`sscanf`/`popen` (`File.cpp`, `System.cpp`)
        // instead of patching the vendored sources to use the `_s` variants.
        build.define("_CRT_SECURE_NO_WARNINGS", None);
    }

    if std::env::var("CARGO_FEATURE_OPENMP").is_ok() {
        // The OpenMP variant needs a system OpenMP runtime (`libgomp`, MSVC's bundled `vcomp`,
        // or Homebrew's `libomp` on macOS, which Apple's clang does not ship). Probe for it
        // rather than assuming it is there, so `--features openmp` degrades to the STL
        // backend (still fully functional) instead of failing the build on a host without
        // one; see docs/design/12-roadmap.md R6 and VENDOR.md.
        if msvc {
            // MSVC ships `vcomp`/`vcompd` with every install and links it automatically for
            // `/openmp`; no probe needed.
            build.define("DDS_THREADS_OPENMP", None).flag("/openmp");
        } else {
            let (flags, link_lib): (&[&str], &str) = if target_os == "macos" {
                (&["-Xpreprocessor", "-fopenmp"], "omp")
            } else {
                (&["-fopenmp"], "gomp")
            };
            if probe_openmp(&target_env, flags, link_lib) {
                build.define("DDS_THREADS_OPENMP", None);
                for flag in flags {
                    build.flag(flag);
                }
                println!("cargo:rustc-link-lib={link_lib}");
            } else {
                println!(
                    "cargo:warning=bridge-dds: no OpenMP runtime found for this toolchain \
                     (Apple clang needs Homebrew's libomp; see VENDOR.md); building the \
                     `openmp` feature with the std::thread backend instead"
                );
            }
        }
    }

    build.compile("dds");
    println!("cargo:rustc-cfg=dds_vendored");
}

/// Tries to compile and link a trivial OpenMP translation unit with `flags` and `link_lib` (a
/// gcc/clang-style `-l` argument). Returns `false` (rather than failing the build) when the
/// toolchain has no usable OpenMP runtime, e.g. Apple clang without Homebrew's `libomp`
/// installed. Not used for MSVC, which links its bundled `vcomp` unconditionally.
fn probe_openmp(target_env: &str, flags: &[&str], link_lib: &str) -> bool {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set for build scripts");
    let probe_dir = Path::new(&out_dir).join("openmp_probe");
    if std::fs::create_dir_all(&probe_dir).is_err() {
        return false;
    }
    let probe_src = probe_dir.join("probe.cpp");
    if std::fs::write(
        &probe_src,
        "#include <omp.h>\nint main() { return omp_get_max_threads() > 0 ? 0 : 1; }\n",
    )
    .is_err()
    {
        return false;
    }

    let mut probe = cc::Build::new();
    probe
        .cpp(true)
        .std(if target_env == "msvc" {
            "c++14"
        } else {
            "c++11"
        })
        .opt_level(0)
        .warnings(false);
    for flag in flags {
        probe.flag(flag);
    }
    let Ok(compiler) = probe.try_get_compiler() else {
        return false;
    };
    let exe = probe_dir.join(if cfg!(windows) {
        "openmp_probe.exe"
    } else {
        "openmp_probe"
    });
    let mut link = compiler.to_command();
    link.arg(&probe_src)
        .args(flags)
        .arg(format!("-l{link_lib}"))
        .arg("-o")
        .arg(&exe);
    matches!(link.status(), Ok(status) if status.success())
}
