//! R10: the description compiler never leaves a `HandConstraint::Custom` in the compiled IR (an
//! unrecognised fragment becomes a lint, not an opaque escape hatch), and a compiled `SystemIR`
//! round-trips through `postcard` byte-for-byte (`docs/design/11-testing.md` §1).
//!
//! **Blocked today**: see `bss_oracle.rs`'s module doc -- every real file is blocked by
//! `compile_description`'s `todo!()` today, so this reports that and returns rather than passing
//! vacuously without saying so.

mod common;

use bridge_constraint::HandConstraint;
use bridge_system::CompileOptions;

fn contains_custom(c: &HandConstraint) -> bool {
    match c {
        HandConstraint::Custom(_) => true,
        HandConstraint::And(terms) | HandConstraint::Or(terms) => terms.iter().any(contains_custom),
        HandConstraint::Not(inner) => contains_custom(inner),
        HandConstraint::Atom(_) => false,
    }
}

#[test]
#[ignore = "needs the vendored systems data (cargo xtask systems fetch)"]
fn compiled_systems_never_contain_custom() {
    let dir = common::systems_dir();
    let opts = CompileOptions::default();

    let files: Vec<_> = [
        "vendor/data/bml-test/data",
        "vendor/data/jdh8",
        "vendor/data/gjp",
    ]
    .iter()
    .flat_map(|sub| common::bml_files(&dir.join(sub)))
    .collect();
    if files.is_empty() {
        eprintln!("no vendored systems data found; skipping");
        return;
    }

    let mut compiled = 0usize;
    for path in &files {
        let Some(ir) = common::compile_guarded(path, &opts) else {
            continue;
        };
        compiled += 1;

        for node in &ir.nodes {
            assert!(
                !contains_custom(&node.constraint),
                "{}: node {:?} ({}) compiled to a HandConstraint::Custom",
                path.display(),
                node.id,
                node.description
            );
        }

        #[cfg(feature = "cache")]
        {
            let bytes = postcard::to_allocvec(&ir).expect("postcard encode");
            let back: bridge_system::SystemIR =
                postcard::from_bytes(&bytes).expect("postcard decode");
            let bytes2 = postcard::to_allocvec(&back).expect("postcard re-encode");
            assert_eq!(
                bytes,
                bytes2,
                "{}: SystemIR does not round-trip byte-for-byte",
                path.display()
            );
        }
    }

    eprintln!(
        "no_custom: {compiled}/{} file(s) compiled and checked (rest blocked by \
         compile_description's todo!())",
        files.len()
    );
}
