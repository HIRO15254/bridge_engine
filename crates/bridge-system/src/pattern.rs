//! Call patterns and their binding semantics.

use bridge_core::{Call, Strain};

/// Whose call a token is: the system owner's side or the opponents' (parenthesised in BML).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Side {
    /// The partnership the system describes.
    Us,
    /// The opponents.
    Them,
}

/// A strain variable. Binds on first use in a path and stays fixed for the subtree.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Var {
    /// `M`: a major not yet bid by either side.
    Major,
    /// `oM`: the other major (requires `M` bound).
    OtherMajor,
    /// `m`: a minor not yet bid by either side.
    Minor,
    /// `om`: the other minor (requires `m` bound).
    OtherMinor,
    /// `X`: any suit not yet bid.
    X,
    /// `Y`: any suit not yet bid, above `X`.
    Y,
    /// `Z`: any suit not yet bid, above `Y`.
    Z,
}

/// A set of strains as a 5-bit mask (`C D H S N`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StrainSet(pub u8);

impl StrainSet {
    /// Empty.
    pub const EMPTY: StrainSet = StrainSet(0);
    /// `red` = diamonds and hearts.
    pub const RED: StrainSet = StrainSet(1 << 1 | 1 << 2);
    /// `black` = clubs and spades.
    pub const BLACK: StrainSet = StrainSet(1 << 0 | 1 << 3);
    /// Hearts and spades.
    pub const MAJORS: StrainSet = StrainSet(1 << 2 | 1 << 3);
    /// Clubs and diamonds.
    pub const MINORS: StrainSet = StrainSet(1 << 0 | 1 << 1);

    /// Whether `strain` is a member.
    pub const fn contains(self, strain: Strain) -> bool {
        (self.0 >> strain.index()) & 1 == 1
    }

    /// This set plus `strain`.
    pub const fn with(self, strain: Strain) -> StrainSet {
        StrainSet(self.0 | 1 << strain.index())
    }

    /// Members in bidding order.
    pub fn iter(self) -> impl Iterator<Item = Strain> {
        Strain::ALL.into_iter().filter(move |s| self.contains(*s))
    }
}

/// A bid level, or `n` for "any level" (extension used by some real files).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Level {
    /// `1..=7`.
    At(u8),
    /// `n`.
    Any,
}

/// A call pattern as written in BML.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CallPattern {
    /// `P`, `D`, `R`, `1C`..`7N` (`1NT` ≡ `1N`); extension: `X`, `XX`.
    Exact(Call),
    /// Literal multi-strain: `1CD`, `2HS`, `3CDH`, `2red`, `2black`. No binding, no filter.
    Strains {
        /// Level.
        level: Level,
        /// Candidate strains.
        strains: StrainSet,
    },
    /// Variable strain: `1M`, `2m`, `1oM`, `2om`, `1X`, `1Y`, `1Z`.
    Var {
        /// Level.
        level: Level,
        /// The variable.
        var: Var,
    },
    /// `1step`, `2steps`: `n` steps above the last bid (either side) in the path.
    Step(u8),
    /// Extension: `2S/3H`, `4D/H` alternatives.
    AnyOf(Vec<CallPattern>),
    /// Extension: an opponents' interference class, kept as a wildcard trie edge.
    Class(OppClass),
}

/// A class of opponents' calls.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum OppClass {
    AnyCall,
    AnyBid,
    AnySuitBid,
    AnyBidAtLevel(u8),
    Double,
    Pass,
}

/// A pattern with its side.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SidedPattern {
    /// Whose call.
    pub side: Side,
    /// The pattern.
    pub pat: CallPattern,
}

/// The binding of variables to strains for one expansion.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Binding {
    /// `m`.
    pub minor: Option<Strain>,
    /// `M`.
    pub major: Option<Strain>,
    /// `X`.
    pub x: Option<Strain>,
    /// `Y`.
    pub y: Option<Strain>,
    /// `Z`.
    pub z: Option<Strain>,
}

impl Binding {
    /// The strain bound to `var`, if any (`oM`/`om` derive from `M`/`m`).
    pub fn get(&self, var: Var) -> Option<Strain> {
        todo!("phase 3")
    }

    /// Candidate strains for an unbound `var` given the strains already bid by either side
    /// (`used`) and the ordering constraints `X < Y < Z`.
    pub fn candidates(&self, var: Var, used: StrainSet) -> Vec<Strain> {
        todo!("phase 3")
    }

    /// A copy with `var` bound to `strain`.
    pub fn bind(self, var: Var, strain: Strain) -> Binding {
        todo!("phase 3")
    }
}
