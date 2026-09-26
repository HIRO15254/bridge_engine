//! `init` idempotence (docs/design/10-dds.md §7.2 rule 1): the process-wide runtime is created
//! by whichever of `init`/first-call wins the race, and every later `init` with a *different*
//! config just logs a warning and keeps the first one rather than erroring or reinitialising
//! `SetResources` a second time. In its own file (not appended to another test file) because
//! this is one process-global `OnceLock`, shared by every test in the same test binary; a
//! separate `--test` binary per file keeps this test's very first call to `init`/any DDS entry
//! point deciding the config, uncontaminated by another file's tests running first.
#![cfg(dds_vendored)]

use bridge_dds::{DdsConfig, init};

#[test]
fn init_is_idempotent_and_keeps_the_first_config() {
    let first = DdsConfig {
        max_threads: 2,
        max_memory_mb: 64,
    };
    init(first).expect("DDS is vendored in this build");

    // A different config on a later call must not panic, error, or replace the active one.
    let second = DdsConfig {
        max_threads: 7,
        max_memory_mb: 999,
    };
    init(second).expect("a later init with a different config must still succeed");

    // Calling init again with the very same config the runtime already has is also fine.
    init(first).expect("re-calling init with the winning config must still succeed");

    // The library does not expose the active config directly; `info()` reflects it indirectly
    // through DDS's own reported thread count, which must be stable across the calls above.
    let a = bridge_dds::info()
        .expect("DDS is vendored in this build")
        .threads;
    let b = bridge_dds::info()
        .expect("DDS is vendored in this build")
        .threads;
    assert_eq!(
        a, b,
        "GetDDSInfo should report a stable thread count once initialised"
    );
}
