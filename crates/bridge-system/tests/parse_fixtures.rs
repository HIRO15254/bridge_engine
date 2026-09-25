//! Snapshot tests for the small, hand-written fixtures under `systems/fixtures/`.
//!
//! Each fixture is parsed and its blocks/lints are snapshotted with `insta`. Run
//! `INSTA_UPDATE=always cargo test -p bridge-system --test parse_fixtures` to (re)generate the
//! `.snap` files, then review the diff by hand before committing.

mod common;

use bridge_system::{lexer, parser};
use common::systems_dir;

fn parse_fixture(name: &str) -> bridge_system::ast::BmlFile {
    let path = systems_dir().join("fixtures").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // Use the bare file name (not the machine-specific absolute path) as the root path so the
    // snapshot content doesn't embed this machine's directory layout.
    let loaded = lexer::load(name, &text, &lexer::FsLoader);
    parser::parse(loaded)
}

macro_rules! fixture_snapshot_test {
    ($test_name:ident, $file_name:expr) => {
        #[test]
        fn $test_name() {
            let file = parse_fixture($file_name);
            insta::assert_debug_snapshot!(stringify!($test_name), (&file.blocks, &file.lints));
        }
    };
}

fixture_snapshot_test!(variables, "variables.bml");
fixture_snapshot_test!(paste, "paste.bml");
fixture_snapshot_test!(seat_vul, "seat_vul.bml");
fixture_snapshot_test!(competitive, "competitive.bml");
fixture_snapshot_test!(continuation, "continuation.bml");
fixture_snapshot_test!(alerts, "alerts.bml");
fixture_snapshot_test!(annotations, "annotations.bml");
