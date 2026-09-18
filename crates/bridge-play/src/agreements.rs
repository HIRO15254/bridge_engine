//! Declared carding agreements.

/// One partnership's carding agreements.
#[derive(Clone, PartialEq, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlayAgreements {
    /// Opening leads.
    pub leads: LeadTable,
    /// Signals.
    pub signals: SignalTable,
    /// Discards.
    pub discards: DiscardTable,
}

/// Lead agreements against suit and notrump contracts.
#[derive(Clone, PartialEq, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LeadTable {
    /// Against suit contracts.
    pub vs_suit: LeadStyle,
    /// Against notrump.
    pub vs_nt: LeadStyle,
}

/// A lead style.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LeadStyle {
    /// Spot-card convention.
    pub spot: SpotLead,
    /// Honour-lead convention.
    pub honors: HonorLeads,
    /// Mass given to the rule (the rest goes to `ANY`); default 0.8.
    pub confidence: f32,
}

impl Default for LeadStyle {
    fn default() -> LeadStyle {
        LeadStyle {
            spot: SpotLead::Unknown,
            honors: HonorLeads::Unknown,
            confidence: 0.8,
        }
    }
}

/// Spot-card lead conventions.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SpotLead {
    /// Fourth highest from length.
    FourthBest,
    /// Third from even, fifth from odd.
    ThirdFifth,
    /// Low = likes the suit, high = does not.
    Attitude,
    /// Rule disabled.
    #[default]
    Unknown,
}

/// Honour-lead conventions.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HonorLeads {
    /// Standard (A from AK, K from KQ, Q from QJ, …).
    Standard,
    /// Rusinow (second highest of touching honours).
    Rusinow,
    /// Jack denies a higher honour, ten implies one.
    JackDenies,
    /// Rule disabled.
    #[default]
    Unknown,
}

/// Signal polarity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Polarity {
    /// High encourages / even count.
    Standard,
    /// Low encourages / odd count (upside-down).
    UpsideDown,
    /// Rule disabled.
    #[default]
    Unknown,
}

/// Signal agreements.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SignalTable {
    /// Attitude on partner's lead.
    pub attitude: Polarity,
    /// Count when declarer leads.
    pub count: Polarity,
    /// Mass given to the rule; default 0.7.
    pub confidence: f32,
}

impl Default for SignalTable {
    fn default() -> SignalTable {
        SignalTable {
            attitude: Polarity::Unknown,
            count: Polarity::Unknown,
            confidence: 0.7,
        }
    }
}

/// First-discard conventions.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FirstDiscard {
    /// Attitude in the discarded suit.
    Attitude,
    /// Lavinthal (suit preference).
    Lavinthal,
    /// Odd encourages, even suit preference.
    OddEven,
    /// Rule disabled.
    #[default]
    Unknown,
}

/// Discard agreements.
#[derive(Clone, PartialEq, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DiscardTable {
    /// First-discard convention.
    pub first: FirstDiscard,
    /// Polarity.
    pub polarity: Polarity,
}
