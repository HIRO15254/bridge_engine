//! Options and reports.

use core::time::Duration;

use bridge_core::Seat;

/// Thread policy. Results are identical either way.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Threads {
    /// Use the `parallel` feature's thread pool when enabled.
    #[default]
    Auto,
    /// Single thread.
    Single,
}

/// Options for [`sample_deals`](crate::sample_deals).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SampleOptions {
    /// Master seed.
    pub seed: u64,
    /// Proposal attempts per sample before it is counted as failed (default 16).
    pub max_attempts_per_sample: u32,
    /// Stop after `n × max_attempt_factor` attempts in total (default 50).
    pub max_attempt_factor: u32,
    /// Thread policy.
    pub threads: Threads,
}

impl Default for SampleOptions {
    fn default() -> SampleOptions {
        SampleOptions {
            seed: 0,
            max_attempts_per_sample: 16,
            max_attempt_factor: 50,
            threads: Threads::Auto,
        }
    }
}

/// Diagnostics of one sampling run (also emitted at `INFO`).
#[derive(Clone, PartialEq, Debug)]
pub struct SampleReport {
    /// Requested deals.
    pub requested: usize,
    /// Produced deals.
    pub produced: usize,
    /// Proposal attempts.
    pub attempts: u64,
    /// `produced / attempts`.
    pub acceptance_rate: f64,
    /// Effective sample size.
    pub ess: f64,
    /// `ess / requested`.
    pub ess_ratio: f64,
    /// Largest log weight.
    pub log_weight_max: f64,
    /// Wall time.
    pub elapsed: Duration,
    /// Warnings.
    pub warnings: Vec<SampleWarning>,
}

/// Something worth knowing about a run.
#[derive(Clone, PartialEq, Debug)]
pub enum SampleWarning {
    /// A seat's constraint contains a custom predicate (rejection sampling).
    CustomConstraint {
        /// The seat.
        seat: Seat,
    },
    /// The effective sample size is far below the request.
    LowEss {
        /// ESS.
        ess: f64,
        /// Requested.
        requested: usize,
    },
    /// Fewer deals than requested were produced.
    Truncated {
        /// Produced.
        produced: usize,
    },
    /// No alternative of a seat is consistent with the known cards.
    EmptySupport {
        /// The seat.
        seat: Seat,
    },
}
