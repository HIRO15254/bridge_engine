//! A caller-owned memo of interpretations (the `SystemIR` itself never caches).

use std::collections::HashMap;
use std::sync::Arc;

use bridge_core::{Auction, Call, Seat, Vulnerability};

use crate::{InterpretOptions, Interpretation, Table, interpret};

/// Memoises [`interpret`](crate::interpret) by `(dealer, vulnerability, calls)`.
#[derive(Default)]
pub struct InterpretCache {
    map: HashMap<(Seat, Vulnerability, Vec<Call>), Arc<Interpretation>>,
}

impl InterpretCache {
    /// An empty cache.
    pub fn new() -> InterpretCache {
        InterpretCache::default()
    }

    /// Returns the cached interpretation or computes and stores it.
    pub fn get_or_interpret(
        &mut self,
        table: &Table,
        auction: &Auction,
        opts: &InterpretOptions,
    ) -> Arc<Interpretation> {
        let key = (
            auction.dealer(),
            auction.vulnerability(),
            auction.calls().to_vec(),
        );
        if let Some(existing) = self.map.get(&key) {
            return existing.clone();
        }
        let interpretation = Arc::new(interpret(table, auction, opts));
        self.map.insert(key, interpretation.clone());
        interpretation
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// `true` when empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}
