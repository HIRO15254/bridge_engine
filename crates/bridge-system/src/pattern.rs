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

impl OppClass {
    /// Whether `call` belongs to this class of opponents' calls. Used by the trie's wildcard
    /// edges (`trie.rs`) to match a concrete opponents' call against a `Class` pattern.
    pub const fn matches(self, call: Call) -> bool {
        match (self, call) {
            (OppClass::AnyCall, _) => true,
            (OppClass::AnyBid, Call::Bid(_)) => true,
            (OppClass::AnySuitBid, Call::Bid(b)) => !matches!(b.strain(), Strain::NoTrump),
            (OppClass::AnyBidAtLevel(n), Call::Bid(b)) => b.level() == n,
            (OppClass::Double, Call::Double) => true,
            (OppClass::Pass, Call::Pass) => true,
            _ => false,
        }
    }
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
        match var {
            Var::Major => self.major,
            Var::Minor => self.minor,
            Var::OtherMajor => self.major.map(other_major),
            Var::OtherMinor => self.minor.map(other_minor),
            Var::X => self.x,
            Var::Y => self.y,
            Var::Z => self.z,
        }
    }

    /// Candidate strains for an unbound `var` given the strains already bid by either side
    /// (`used`) and the ordering constraints `X < Y < Z`.
    ///
    /// `oM`/`om` have no fresh candidates (they derive from an already-bound `M`/`m`; the caller
    /// must have checked that with [`Binding::get`] first and dropped the row otherwise, per
    /// `Lint::UnboundOther`), so they yield an empty list here.
    pub fn candidates(&self, var: Var, used: StrainSet) -> Vec<Strain> {
        let domain = match var {
            Var::Major => StrainSet::MAJORS,
            Var::Minor => StrainSet::MINORS,
            Var::X | Var::Y | Var::Z => StrainSet(StrainSet::MINORS.0 | StrainSet::MAJORS.0),
            Var::OtherMajor | Var::OtherMinor => return Vec::new(),
        };
        // X < Y < Z, checked in both directions: a bound variable constrains every other
        // variable's candidates, not just the one immediately below or above it in the X, Y, Z
        // order. For each of X, Y, Z the lower bound is the highest already-bound variable that
        // must be below it, and the upper bound is the lowest already-bound variable that must
        // be above it.
        let (lower_bound, upper_bound) = match var {
            Var::X => (None, min_strain(self.y, self.z)),
            Var::Y => (self.x, self.z),
            Var::Z => (max_strain(self.x, self.y), None),
            _ => (None, None),
        };
        domain
            .iter()
            .filter(|&s| !used.contains(s))
            .filter(|&s| lower_bound.is_none_or(|lb| s.index() > lb.index()))
            .filter(|&s| upper_bound.is_none_or(|ub| s.index() < ub.index()))
            .collect()
    }

    /// A copy with `var` bound to `strain`. Binding `oM`/`om` is a no-op: they are never fresh
    /// variables, only derived views of `M`/`m` (see [`Binding::get`]).
    #[must_use]
    pub fn bind(self, var: Var, strain: Strain) -> Binding {
        let mut b = self;
        match var {
            Var::Major => b.major = Some(strain),
            Var::Minor => b.minor = Some(strain),
            Var::X => b.x = Some(strain),
            Var::Y => b.y = Some(strain),
            Var::Z => b.z = Some(strain),
            Var::OtherMajor | Var::OtherMinor => {}
        }
        b
    }
}

/// The lower of two optional strains (by bidding-order index), or the one that is `Some`, or
/// `None` if both are unbound.
fn min_strain(a: Option<Strain>, b: Option<Strain>) -> Option<Strain> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a.index() <= b.index() { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// The higher of two optional strains (by bidding-order index), or the one that is `Some`, or
/// `None` if both are unbound.
fn max_strain(a: Option<Strain>, b: Option<Strain>) -> Option<Strain> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a.index() >= b.index() { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// The major other than `m` (`m` must itself be a major).
fn other_major(m: Strain) -> Strain {
    if m == Strain::Hearts {
        Strain::Spades
    } else {
        Strain::Hearts
    }
}

/// The minor other than `m` (`m` must itself be a minor).
fn other_minor(m: Strain) -> Strain {
    if m == Strain::Clubs {
        Strain::Diamonds
    } else {
        Strain::Clubs
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;

    #[test]
    fn get_returns_bound_strain() {
        let b = Binding::default().bind(Var::Major, Strain::Spades);
        assert_eq!(b.get(Var::Major), Some(Strain::Spades));
        assert_eq!(b.get(Var::Minor), None);
    }

    #[test]
    fn other_major_and_minor_derive_from_the_base() {
        let b = Binding::default()
            .bind(Var::Major, Strain::Hearts)
            .bind(Var::Minor, Strain::Diamonds);
        assert_eq!(b.get(Var::OtherMajor), Some(Strain::Spades));
        assert_eq!(b.get(Var::OtherMinor), Some(Strain::Clubs));
    }

    #[test]
    fn other_major_is_none_when_major_unbound() {
        let b = Binding::default();
        assert_eq!(b.get(Var::OtherMajor), None);
        assert_eq!(b.get(Var::OtherMinor), None);
    }

    #[test]
    fn candidates_major_excludes_used_strains() {
        let b = Binding::default();
        let used = StrainSet::EMPTY.with(Strain::Hearts);
        assert_eq!(b.candidates(Var::Major, used), vec![Strain::Spades]);
    }

    #[test]
    fn candidates_minor_domain_is_clubs_and_diamonds() {
        let b = Binding::default();
        assert_eq!(
            b.candidates(Var::Minor, StrainSet::EMPTY),
            vec![Strain::Clubs, Strain::Diamonds]
        );
    }

    #[test]
    fn candidates_x_y_z_respect_ordering() {
        let b = Binding::default();
        // No binding yet: X ranges over all four suits.
        assert_eq!(
            b.candidates(Var::X, StrainSet::EMPTY),
            vec![
                Strain::Clubs,
                Strain::Diamonds,
                Strain::Hearts,
                Strain::Spades
            ]
        );

        let b = b.bind(Var::X, Strain::Diamonds);
        // Y must be above X (Diamonds): Hearts, Spades only.
        assert_eq!(
            b.candidates(Var::Y, StrainSet::EMPTY),
            vec![Strain::Hearts, Strain::Spades]
        );

        let b = b.bind(Var::Y, Strain::Hearts);
        // Z must be above Y (Hearts): Spades only.
        assert_eq!(b.candidates(Var::Z, StrainSet::EMPTY), vec![Strain::Spades]);
    }

    #[test]
    fn candidates_x_is_bounded_above_by_bound_y() {
        // Y bound to Hearts: X must be below Y, i.e. Clubs or Diamonds only.
        let b = Binding::default().bind(Var::Y, Strain::Hearts);
        assert_eq!(
            b.candidates(Var::X, StrainSet::EMPTY),
            vec![Strain::Clubs, Strain::Diamonds]
        );
    }

    #[test]
    fn candidates_x_is_bounded_above_by_bound_z_when_y_unbound() {
        // Z bound to Hearts, Y unbound: X must still be below Z (X < Y < Z transitively).
        let b = Binding::default().bind(Var::Z, Strain::Hearts);
        assert_eq!(
            b.candidates(Var::X, StrainSet::EMPTY),
            vec![Strain::Clubs, Strain::Diamonds]
        );
    }

    #[test]
    fn candidates_z_is_bounded_below_by_bound_x_when_y_unbound() {
        // X bound to Diamonds, Y unbound: Z must be above X, i.e. Hearts or Spades.
        let b = Binding::default().bind(Var::X, Strain::Diamonds);
        assert_eq!(
            b.candidates(Var::Z, StrainSet::EMPTY),
            vec![Strain::Hearts, Strain::Spades]
        );
    }

    #[test]
    fn candidates_exclude_strains_already_bid_by_either_side() {
        let b = Binding::default();
        let used = StrainSet::EMPTY.with(Strain::Clubs).with(Strain::Diamonds);
        assert_eq!(
            b.candidates(Var::X, used),
            vec![Strain::Hearts, Strain::Spades]
        );
    }

    #[test]
    fn candidates_other_major_and_minor_are_empty() {
        let b = Binding::default().bind(Var::Major, Strain::Hearts);
        assert!(b.candidates(Var::OtherMajor, StrainSet::EMPTY).is_empty());
    }

    #[test]
    fn bind_is_immutable_and_returns_a_copy() {
        let b0 = Binding::default();
        let b1 = b0.bind(Var::Minor, Strain::Clubs);
        assert_eq!(b0.get(Var::Minor), None);
        assert_eq!(b1.get(Var::Minor), Some(Strain::Clubs));
    }
}
