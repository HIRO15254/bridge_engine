//! Manual `serde` implementations for the newtypes.
//!
//! Human-readable formats (JSON, TOML) get the `Display` strings; binary formats (postcard,
//! bincode) get the raw integers. `Deserialize` always validates (`Card < 52`, `Hand` bits below
//! 2^52, `Deal::new`, `ShapeSet` bits below 560). `Auction` deserializes through a private
//! mirror struct and `Auction::from_calls`, so an illegal auction can never be constructed.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Auction, Bid, Call, Card, Deal, Hand, Holding, Seat, Shape, ShapeSet, Vulnerability};

impl Serialize for Card {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        todo!("phase 1")
    }
}

impl<'de> Deserialize<'de> for Card {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Card, D::Error> {
        todo!("phase 1")
    }
}

impl Serialize for Holding {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        todo!("phase 1")
    }
}

impl<'de> Deserialize<'de> for Holding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Holding, D::Error> {
        todo!("phase 1")
    }
}

impl Serialize for Hand {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        todo!("phase 1")
    }
}

impl<'de> Deserialize<'de> for Hand {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Hand, D::Error> {
        todo!("phase 1")
    }
}

impl Serialize for Shape {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        todo!("phase 1")
    }
}

impl<'de> Deserialize<'de> for Shape {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Shape, D::Error> {
        todo!("phase 1")
    }
}

impl Serialize for ShapeSet {
    /// Human-readable: four ranges when [`ShapeSet::factor`] succeeds, else a list of classes
    /// when the set is a union of whole classes, else a 140-hex-digit bitmap. Binary: `[u64; 9]`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        todo!("phase 2")
    }
}

impl<'de> Deserialize<'de> for ShapeSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ShapeSet, D::Error> {
        todo!("phase 2")
    }
}

impl Serialize for Bid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        todo!("phase 1")
    }
}

impl<'de> Deserialize<'de> for Bid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Bid, D::Error> {
        todo!("phase 1")
    }
}

impl Serialize for Deal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        todo!("phase 1")
    }
}

impl<'de> Deserialize<'de> for Deal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Deal, D::Error> {
        todo!("phase 1")
    }
}

/// Wire form of [`Auction`]; deserialization re-validates through [`Auction::from_calls`].
#[derive(Serialize, Deserialize)]
struct AuctionRepr {
    dealer: Seat,
    vulnerability: Vulnerability,
    calls: Vec<Call>,
}

impl Serialize for Auction {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        AuctionRepr {
            dealer: self.dealer(),
            vulnerability: self.vulnerability(),
            calls: self.calls().to_vec(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Auction {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Auction, D::Error> {
        let repr = AuctionRepr::deserialize(deserializer)?;
        Auction::from_calls(repr.dealer, repr.vulnerability, repr.calls)
            .map_err(serde::de::Error::custom)
    }
}
