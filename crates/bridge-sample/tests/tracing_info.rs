//! `sample_deals` always emits one `INFO` event with the run's ESS (09-sample.md §2.3 step 5,
//! 12-roadmap.md task 5.3), carrying the same numbers as the returned `SampleReport`.

use std::sync::{Arc, Mutex};

use bridge_bidding::Interpretation;
use bridge_constraint::HandConstraint;
use bridge_sample::{
    KnownCards, SampleContext, SampleOptions, Threads, UniformProposal, sample_deals,
};

/// What the recorder saw of one event.
#[derive(Default, Clone, Debug)]
struct Seen {
    level: Option<tracing::Level>,
    message: String,
    ess: Option<f64>,
    ess_ratio: Option<f64>,
    requested: Option<u64>,
    produced: Option<u64>,
}

struct Visitor<'a>(&'a mut Seen);

impl tracing::field::Visit for Visitor<'_> {
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        match field.name() {
            "ess" => self.0.ess = Some(value),
            "ess_ratio" => self.0.ess_ratio = Some(value),
            _ => {}
        }
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "requested" => self.0.requested = Some(value),
            "produced" => self.0.produced = Some(value),
            _ => {}
        }
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn core::fmt::Debug) {
        if field.name() == "message" {
            self.0.message = format!("{value:?}");
        }
    }
}

/// A minimal `tracing::Subscriber` recording every event from `bridge_sample` (without the
/// `tracing-subscriber` crate, which is not a workspace dependency).
struct Recorder(Arc<Mutex<Vec<Seen>>>);

impl tracing::Subscriber for Recorder {
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if !event.metadata().target().starts_with("bridge_sample") {
            return;
        }
        let mut seen = Seen {
            level: Some(*event.metadata().level()),
            ..Seen::default()
        };
        event.record(&mut Visitor(&mut seen));
        self.0.lock().expect("recorder lock").push(seen);
    }
    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}

#[test]
fn sample_deals_emits_the_info_line_with_ess() {
    let interpretation = Interpretation {
        seats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        per_call: Vec::new(),
        divergence: None,
    };
    let play_constraints = [
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
        HandConstraint::ANY,
    ];
    let ctx = SampleContext {
        known: KnownCards::EMPTY,
        interpretation: &interpretation,
        play_constraints: &play_constraints,
        play_soft: None,
        bidding: None,
    };
    let opts = SampleOptions {
        seed: 11,
        threads: Threads::Single,
        ..SampleOptions::default()
    };

    // `tracing`'s per-callsite interest cache is process-global; see
    // `bridge-constraint/tests/sampler_rejection.rs` for why a rebuild plus one retry makes this
    // independent of what other threads registered first.
    for attempt in 0..2 {
        let events = Arc::new(Mutex::new(Vec::new()));
        let report = tracing::subscriber::with_default(Recorder(events.clone()), || {
            tracing::callsite::rebuild_interest_cache();
            sample_deals(&ctx, &UniformProposal, 50, &opts)
                .expect("unconstrained sampling succeeds")
                .1
        });
        let events = events.lock().expect("recorder lock").clone();
        let Some(info) = events
            .iter()
            .find(|e| e.message.contains("deal sampling finished"))
        else {
            assert_eq!(attempt, 0, "no INFO line from sample_deals: {events:?}");
            continue;
        };
        assert_eq!(info.level, Some(tracing::Level::INFO));
        assert_eq!(info.ess, Some(report.ess));
        assert_eq!(info.ess_ratio, Some(report.ess_ratio));
        assert_eq!(info.requested, Some(50));
        assert_eq!(info.produced, Some(report.produced as u64));
        return;
    }
}
