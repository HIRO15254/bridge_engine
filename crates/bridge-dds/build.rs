//! Compiles the vendored DDS 2.9.0 sources with `cc`.
//!
//! The sources are not committed; `cargo xtask dds vendor` downloads them into
//! `vendor/dds-2.9.0/`. When they are absent the crate still builds, with the FFI layer
//! compiled out (`cfg(dds_vendored)` unset) so that the workspace checks on every machine.

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
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        return; // lib.rs emits a compile_error! for wasm32
    }

    let src = Path::new("vendor/dds-2.9.0/src");
    if !src.join("dds.cpp").exists() {
        println!(
            "cargo:warning=bridge-dds: DDS sources not vendored; run `cargo xtask dds vendor` (FFI disabled)"
        );
        return;
    }

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++11")
        .opt_level(3)
        .include("vendor/dds-2.9.0/include")
        .include(src)
        .files(SOURCES.iter().map(|f| src.join(format!("{f}.cpp"))))
        .file("src/layout_probe.cpp")
        .define("DDS_THREADS_STL", None)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-deprecated-declarations");
    if std::env::var("CARGO_FEATURE_OPENMP").is_ok() {
        build.define("DDS_THREADS_OPENMP", None).flag("-fopenmp");
        println!("cargo:rustc-link-lib=gomp");
    }
    build.compile("dds");
    println!("cargo:rustc-cfg=dds_vendored");
}
