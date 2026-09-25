//! Manual `serde` implementations for the newtypes.
//!
//! Human-readable formats (JSON, TOML) get the `Display` strings; binary formats (postcard,
//! bincode) get the raw integers. `Deserialize` always validates (`Card < 52`, `Hand` bits below
//! 2^52, `Deal::new`, `ShapeSet` bits below 560). `Auction` deserializes through a private
//! mirror struct and `Auction::from_calls`, so an illegal auction can never be constructed.

use core::fmt;
use core::str::FromStr;

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    Auction, Bid, Call, Card, Deal, Hand, Holding, Seat, Shape, ShapeClass, ShapeSet, Suit,
    Vulnerability,
};

/// Deserializes a `Display` string (human-readable formats) through `FromStr`.
fn from_text<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    let text = String::deserialize(deserializer)?;
    text.parse().map_err(de::Error::custom)
}

impl Serialize for Card {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.collect_str(self)
        } else {
            serializer.serialize_u8(self.index())
        }
    }
}

impl<'de> Deserialize<'de> for Card {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Card, D::Error> {
        if deserializer.is_human_readable() {
            from_text(deserializer)
        } else {
            let i = u8::deserialize(deserializer)?;
            Card::from_index(i)
                .ok_or_else(|| de::Error::custom(format!("card index {i} is not below 52")))
        }
    }
}

impl Serialize for Holding {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.collect_str(self)
        } else {
            serializer.serialize_u16(self.bits())
        }
    }
}

impl<'de> Deserialize<'de> for Holding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Holding, D::Error> {
        if deserializer.is_human_readable() {
            from_text(deserializer)
        } else {
            let bits = u16::deserialize(deserializer)?;
            Holding::from_bits(bits)
                .ok_or_else(|| de::Error::custom(format!("holding {bits:#x} has bits above 12")))
        }
    }
}

impl Serialize for Hand {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.collect_str(self)
        } else {
            serializer.serialize_u64(self.bits())
        }
    }
}

impl<'de> Deserialize<'de> for Hand {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Hand, D::Error> {
        if deserializer.is_human_readable() {
            from_text(deserializer)
        } else {
            let bits = u64::deserialize(deserializer)?;
            hand_from_bits(bits).map_err(de::Error::custom)
        }
    }
}

fn hand_from_bits(bits: u64) -> Result<Hand, String> {
    Hand::from_bits(bits).ok_or_else(|| format!("hand {bits:#x} has bits above 51"))
}

impl Serialize for Shape {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.collect_str(self)
        } else {
            serializer.serialize_u16(self.bits())
        }
    }
}

impl<'de> Deserialize<'de> for Shape {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Shape, D::Error> {
        if deserializer.is_human_readable() {
            from_text(deserializer)
        } else {
            let bits = u16::deserialize(deserializer)?;
            let lens = [
                (bits & 0xF) as u8,
                ((bits >> 4) & 0xF) as u8,
                ((bits >> 8) & 0xF) as u8,
                ((bits >> 12) & 0xF) as u8,
            ];
            if lens.iter().any(|&l| l > 13) {
                return Err(de::Error::custom(format!(
                    "shape {bits:#x} has a suit length above 13"
                )));
            }
            Ok(Shape::from_lens(lens))
        }
    }
}

impl Serialize for Bid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.collect_str(self)
        } else {
            serializer.serialize_u8(self.index())
        }
    }
}

impl<'de> Deserialize<'de> for Bid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Bid, D::Error> {
        if deserializer.is_human_readable() {
            from_text(deserializer)
        } else {
            let i = u8::deserialize(deserializer)?;
            Bid::from_index(i)
                .ok_or_else(|| de::Error::custom(format!("bid index {i} is not below 35")))
        }
    }
}

impl Serialize for Deal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.collect_str(self)
        } else {
            self.hands().map(Hand::bits).serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for Deal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Deal, D::Error> {
        if deserializer.is_human_readable() {
            from_text(deserializer)
        } else {
            let words = <[u64; 4]>::deserialize(deserializer)?;
            let mut hands = [Hand::EMPTY; 4];
            for (hand, bits) in hands.iter_mut().zip(words) {
                *hand = hand_from_bits(bits).map_err(de::Error::custom)?;
            }
            Deal::new(hands).map_err(de::Error::custom)
        }
    }
}

/// Number of hex digits of the 560-bit bitmap form of a [`ShapeSet`].
const BITMAP_HEX_DIGITS: usize = 140;

/// The bitmap form: 560 bits as one big-endian hexadecimal number (word 8 first).
fn shape_set_to_hex(set: ShapeSet) -> String {
    use core::fmt::Write as _;
    let words = set.words();
    let mut out = String::with_capacity(BITMAP_HEX_DIGITS);
    let _ = write!(out, "{:012x}", words[8]);
    for w in words[..8].iter().rev() {
        let _ = write!(out, "{w:016x}");
    }
    out
}

fn shape_set_from_hex(s: &str) -> Result<ShapeSet, String> {
    if s.len() != BITMAP_HEX_DIGITS || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!(
            "expected {BITMAP_HEX_DIGITS} hex digits, found {s:?}"
        ));
    }
    let mut words = [0u64; 9];
    words[8] = u64::from_str_radix(&s[..12], 16).map_err(|e| e.to_string())?;
    for (k, w) in words[..8].iter_mut().rev().enumerate() {
        let start = 12 + 16 * k;
        *w = u64::from_str_radix(&s[start..start + 16], 16).map_err(|e| e.to_string())?;
    }
    ShapeSet::from_words(words).ok_or_else(|| "shape set has bits at or above 560".to_string())
}

/// The range form: `C2-5 D0-13 H4-4 S3-13`. Missing suits default to `0-13`.
fn shape_set_from_ranges(s: &str) -> Result<ShapeSet, String> {
    let mut lens = [(0u8, 13u8); 4];
    for token in s.split_whitespace() {
        let mut chars = token.chars();
        let suit = chars
            .next()
            .and_then(|c| c.to_string().parse::<Suit>().ok())
            .ok_or_else(|| format!("expected a suit letter in {token:?}"))?;
        let (lo, hi) = chars
            .as_str()
            .split_once('-')
            .ok_or_else(|| format!("expected lo-hi in {token:?}"))?;
        let parse = |t: &str| {
            t.trim()
                .parse::<u8>()
                .map_err(|e| format!("{token:?}: {e}"))
        };
        lens[suit.index() as usize] = (parse(lo)?, parse(hi)?);
    }
    Ok(ShapeSet::from_suit_lens(lens))
}

/// The whole classes making up `set` (sorted, longest suit first), if it is a union of whole
/// classes.
fn shape_set_classes(set: ShapeSet) -> Option<Vec<ShapeClass>> {
    let mask = set.classes();
    let mut classes: Vec<ShapeClass> = (0..39u8)
        .filter(|i| (mask >> i) & 1 == 1)
        .map(ShapeClass::from_index)
        .collect();
    classes.sort();
    (ShapeSet::from_classes(&classes) == set).then_some(classes)
}

impl Serialize for ShapeSet {
    /// Human-readable: four ranges when [`ShapeSet::factor`] succeeds, else a list of classes
    /// when the set is a union of whole classes, else a 140-hex-digit bitmap. Binary: `[u64; 9]`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if !serializer.is_human_readable() {
            return self.words().serialize(serializer);
        }
        if let Some(r) = self.factor() {
            let text = format!(
                "C{}-{} D{}-{} H{}-{} S{}-{}",
                r[0].start(),
                r[0].end(),
                r[1].start(),
                r[1].end(),
                r[2].start(),
                r[2].end(),
                r[3].start(),
                r[3].end(),
            );
            return serializer.serialize_str(&text);
        }
        if let Some(classes) = shape_set_classes(*self) {
            return serializer.collect_seq(classes.iter().map(ToString::to_string));
        }
        serializer.serialize_str(&shape_set_to_hex(*self))
    }
}

struct ShapeSetVisitor;

impl<'de> Visitor<'de> for ShapeSetVisitor {
    type Value = ShapeSet;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("four suit-length ranges, a list of shape classes, or a 140-digit hex bitmap")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<ShapeSet, E> {
        let v = v.trim();
        let parsed = if v.contains('-') {
            shape_set_from_ranges(v)
        } else {
            shape_set_from_hex(v)
        };
        parsed.map_err(E::custom)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<ShapeSet, A::Error> {
        let mut classes = Vec::new();
        while let Some(name) = seq.next_element::<String>()? {
            classes.push(name.parse::<ShapeClass>().map_err(de::Error::custom)?);
        }
        Ok(ShapeSet::from_classes(&classes))
    }
}

impl<'de> Deserialize<'de> for ShapeSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ShapeSet, D::Error> {
        if deserializer.is_human_readable() {
            deserializer.deserialize_any(ShapeSetVisitor)
        } else {
            let words = <[u64; 9]>::deserialize(deserializer)?;
            ShapeSet::from_words(words)
                .ok_or_else(|| de::Error::custom("shape set has bits at or above 560"))
        }
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
