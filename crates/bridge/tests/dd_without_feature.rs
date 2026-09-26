//! `bridge::dd::dds()` compiles and returns `None` when the `dds` feature is off, so that a
//! caller can always write `if let Some(solver) = bridge::dd::dds() { .. }` regardless of which
//! way this crate was built (docs/design/12-roadmap.md 5.7). The complementary case (feature on)
//! is `tests/dds.rs`'s `dds_none_iff_unavailable`; this file is gated the other way so the two
//! never both compile into the same build and silently test the same thing twice.
#![cfg(not(feature = "dds"))]

use bridge::dd::dds;

#[test]
fn dds_is_none_when_the_dds_feature_is_disabled() {
    assert!(
        dds().is_none(),
        "bridge::dd::dds() must return None when this crate was built without the `dds` feature"
    );
}
