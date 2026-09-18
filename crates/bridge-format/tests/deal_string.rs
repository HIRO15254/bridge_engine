//! Deal string round trip.

mod common;

use bridge_core::Deal;
use bridge_format::deal_string;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn round_trip(deal in common::arb_deal(), first in common::arb_seat()) {
        let text = deal_string::write(&deal, first);
        let prefix = format!("{first}:");
        prop_assert!(text.starts_with(&prefix));
        let parsed = deal_string::parse(&text).unwrap();
        prop_assert_eq!(parsed.complete(), Some(deal));
        prop_assert_eq!(deal_string::write(&parsed.complete().unwrap(), first), text.clone());
        // `bridge-core`'s own reader agrees, and lower case is accepted.
        prop_assert_eq!(text.parse::<Deal>().unwrap(), deal);
        let lower = deal_string::parse(&text.to_ascii_lowercase()).unwrap();
        prop_assert_eq!(lower.complete(), Some(deal));
    }
}
