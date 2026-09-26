//! Options for [`crate::advise`].

use bridge_bidding::{InterpretOptions, PolicyParams};
use bridge_sample::SampleOptions;

/// How to rank the lead groups (`14-lead.md` §3).
///
/// Every [`crate::LeadScore`] always reports `mean_defence_tricks`, `std_error` and
/// `set_probability` regardless of which scoring is chosen: `scoring` only changes the sort
/// order (and hence `rank`), never which statistics are computed.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum LeadScoring {
    /// Descending mean defence tricks (the default).
    #[default]
    Tricks,
    /// Descending probability of defeating the contract.
    SetProbability,
    /// Ascending expected declarer score (the lead most damaging to declarer sorts first).
    ///
    /// This is the closest cheap approximation to matchpoint/IMP scoring available without a
    /// full double-dummy solve of every contract at the table (`14-lead.md` §3.1, §5 item 3).
    /// Vulnerability is not a caller-supplied field here: `advise` derives it from the auction
    /// itself (`auction.vulnerability().is_vulnerable(declarer)`), since the auction already
    /// carries it precisely and a duplicate field would only invite it to disagree.
    Score,
}

/// Options for [`crate::advise`].
#[derive(Clone, Copy, Debug)]
pub struct LeadOptions {
    /// Number of deals to sample (default 200).
    pub samples: usize,
    /// The single seed controlling the whole run.
    ///
    /// `advise` builds the [`SampleOptions`] it actually passes to `sample_deals` as
    /// `SampleOptions { seed, ..self.sample }`, so callers set `seed` once here rather than
    /// keeping it in sync with `sample.seed` themselves.
    pub seed: u64,
    /// The bidding policy the auction is weighted by (`docs/design/14-lead.md` §3; default
    /// [`PolicyParams::human`]: real tables leave the system for natural calls, and the
    /// system-only preset would read every such call as a near-impossible deviation).
    pub policy: PolicyParams,
    /// Options for [`bridge_bidding::interpret`].
    ///
    /// Its `policy` and `implicit_pass` are overridden with the likelihood's own
    /// ([`InterpretOptions::for_context`]), so the interpretation is always the mirror of the
    /// policy the deals are weighted by; the other fields are used as given.
    pub interpret: InterpretOptions,
    /// Options for [`bridge_sample::sample_deals`] other than `seed` (attempts, thread policy).
    pub sample: SampleOptions,
    /// Number of lead groups to return (default 3).
    pub top_k: usize,
    /// How to rank the lead groups.
    pub scoring: LeadScoring,
}

impl Default for LeadOptions {
    fn default() -> LeadOptions {
        LeadOptions {
            samples: 200,
            seed: 0,
            policy: PolicyParams::human(),
            interpret: InterpretOptions::default(),
            sample: SampleOptions::default(),
            top_k: 3,
            scoring: LeadScoring::default(),
        }
    }
}
