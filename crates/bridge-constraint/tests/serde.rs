//! `HandConstraint`'s manual `serde` impl: an externally-tagged tree that round-trips
//! `Atom`/`Or`/`And`/`Not`, and refuses to serialize a `Custom` node (it is not data).

#![cfg(feature = "serde")]

mod common;

use bridge_constraint::HandConstraint;
use common::{arb_constraint_no_custom, arb_hand13, custom_pred};
use proptest::prelude::*;

proptest! {
    /// Serializing and deserializing a `Custom`-free tree preserves its behaviour.
    #[test]
    fn round_trip_preserves_behaviour(
        c in arb_constraint_no_custom(3),
        hands in prop::collection::vec(arb_hand13(), 20),
    ) {
        let json = serde_json::to_string(&c).expect("Custom-free tree serializes");
        let back: HandConstraint = serde_json::from_str(&json).expect("round trip deserializes");
        for hand in hands {
            prop_assert_eq!(c.satisfies(hand), back.satisfies(hand));
        }
    }
}

#[test]
fn custom_cannot_be_serialized() {
    let c = HandConstraint::Custom(custom_pred(0));
    let err = serde_json::to_string(&c).expect_err("Custom is not data");
    assert!(
        err.to_string().contains("Custom"),
        "error should mention why: {err}"
    );
}
