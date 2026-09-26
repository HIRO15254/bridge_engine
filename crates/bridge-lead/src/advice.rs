//! The advisor's output.

use bridge_core::{Card, Contract, Seat};
use bridge_sample::SampleReport;

/// One ranked lead (a group of cards that scored identically in every sample).
#[derive(Clone, Debug)]
pub struct LeadScore {
    /// The group's representative: the highest-ranking card among cards that scored identically
    /// in every sample (by bridge convention, the touching honours are led from the top).
    pub card: Card,
    /// The other cards that scored identically to `card` in every sample (touching honours,
    /// equal spot cards), excluding `card` itself, highest rank first.
    pub equivalents: Vec<Card>,
    /// Mean defence tricks after this lead, under the self-normalised importance weights.
    pub mean_defence_tricks: f64,
    /// Standard error of `mean_defence_tricks` (weighted standard deviation over `sqrt(ess)`).
    pub std_error: f64,
    /// `P(defence tricks >= 8 - level)`: the probability this lead is part of a defence that
    /// defeats the contract (`14-lead.md` §3 step 7).
    pub set_probability: f64,
    /// 1-based rank under the requested [`crate::LeadScoring`].
    pub rank: usize,
}

/// The result of [`crate::advise`].
#[derive(Clone, Debug)]
pub struct LeadAdvice {
    /// The final contract.
    pub contract: Contract,
    /// The declaring side's player who named the contract's strain first.
    pub declarer: Seat,
    /// The opening leader (`contract.leader()`).
    pub leader: Seat,
    /// Ranked leads, truncated to `opts.top_k` groups.
    pub leads: Vec<LeadScore>,
    /// The full sampling report (produced count, ESS, warnings, timing).
    pub sample_report: SampleReport,
    /// Convenience copy of `sample_report.produced`.
    pub samples_used: usize,
    /// Convenience copy of `sample_report.ess`.
    pub ess: f64,
}
