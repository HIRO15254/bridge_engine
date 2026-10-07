//! A per-thread, bounded memo of the hand-independent data of a position (07-bidding.md §6.2).
//!
//! Everything `choose_bid`, `call_distribution` and the mirror compute about a position before
//! looking at a hand depends only on the table, the prefix and the implicit-pass policy: the
//! system candidates of [`crate::choose::enumerate_position`] (a trie resolve, possibly a lenient
//! one, and the children with their legality) and, at a natural position, the ranked natural
//! candidates with their partner context and per-call regions
//! ([`crate::exclusion::NaturalPos`]). This memo keeps both per prefix, so an auction's
//! positions are resolved once per thread instead of once per call, per interpretation and per
//! policy evaluation. A hit returns exactly what a recomputation would.
//!
//! The key is `(addresses of the table's four systems and natural engine, implicit pass,
//! dealer, vulnerability, calls)`; every entry holds `Weak` references to those five
//! allocations, so none of their addresses can be reused by another table while the entry
//! exists. The memo keeps two generations of [`GENERATION`] entries each (an approximate
//! least-recently-used policy: a hit in the older generation moves the entry to the newer one,
//! and a full newer generation replaces the older one).

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;
use std::sync::{Arc, Weak};

use bridge_core::{Auction, Call, Seat, Vulnerability};

use crate::choose::PositionCore;
use crate::exclusion::NaturalPos;
use crate::{ImplicitPass, NaturalInference, SystemIR, Table};

/// Entries per generation (two generations are kept).
const GENERATION: usize = 1024;

/// The memoised data of one prefix.
pub(crate) struct PrefixEntry {
    tables: [usize; 5],
    complement: bool,
    dealer: Seat,
    vul: Vulnerability,
    calls: Vec<Call>,
    /// Keeps the keyed allocations alive (see the module documentation).
    _keep: ([Weak<SystemIR>; 4], Weak<NaturalInference>),
    /// The system candidates of the position.
    pub(crate) core: PositionCore,
    /// The natural data of the position under the table's own natural engine, once computed.
    pub(crate) natural: RefCell<Option<Rc<NaturalPos>>>,
}

impl PrefixEntry {
    fn matches(&self, tables: &[usize; 5], complement: bool, auction: &Auction) -> bool {
        self.tables == *tables
            && self.complement == complement
            && self.dealer == auction.dealer()
            && self.vul == auction.vulnerability()
            && self.calls == auction.calls()
    }
}

/// A hasher for keys that are already hashes.
#[derive(Default)]
struct PassHasher(u64);

impl Hasher for PassHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = mix(self.0, u64::from(b));
        }
    }

    fn write_u64(&mut self, x: u64) {
        self.0 = x;
    }
}

type Map = HashMap<u64, Rc<PrefixEntry>, BuildHasherDefault<PassHasher>>;

#[derive(Default)]
struct Memo {
    current: Map,
    previous: Map,
}

thread_local! {
    static MEMO: RefCell<Memo> = RefCell::new(Memo::default());
}

fn mix(h: u64, x: u64) -> u64 {
    (h.rotate_left(5) ^ x).wrapping_mul(0x517c_c1b7_2722_0a95)
}

fn addresses(table: &Table) -> [usize; 5] {
    let addr = |s: &Arc<SystemIR>| Arc::as_ptr(s) as usize;
    [
        addr(&table.systems[0]),
        addr(&table.systems[1]),
        addr(&table.systems[2]),
        addr(&table.systems[3]),
        Arc::as_ptr(&table.natural) as usize,
    ]
}

fn hash(tables: &[usize; 5], complement: bool, auction: &Auction) -> u64 {
    let mut h = 0u64;
    for &t in tables {
        h = mix(h, t as u64);
    }
    h = mix(
        h,
        u64::from(complement)
            | u64::from(auction.dealer().index()) << 1
            | u64::from(auction.vulnerability().index()) << 3,
    );
    for chunk in auction.calls().chunks(8) {
        let mut w = 0u64;
        for (i, c) in chunk.iter().enumerate() {
            w |= u64::from(c.index()) << (8 * i);
        }
        h = mix(h, w);
    }
    mix(h, auction.len() as u64)
}

impl Memo {
    fn get(
        &mut self,
        h: u64,
        tables: &[usize; 5],
        complement: bool,
        auction: &Auction,
    ) -> Option<Rc<PrefixEntry>> {
        if let Some(e) = self.current.get(&h) {
            return e.matches(tables, complement, auction).then(|| e.clone());
        }
        let e = self.previous.remove(&h)?;
        if !e.matches(tables, complement, auction) {
            return None;
        }
        self.insert(h, e.clone());
        Some(e)
    }

    fn insert(&mut self, h: u64, e: Rc<PrefixEntry>) {
        if self.current.len() >= GENERATION {
            let full = std::mem::replace(
                &mut self.current,
                Map::with_capacity_and_hasher(GENERATION, BuildHasherDefault::default()),
            );
            // Dropping the old generation frees its entries here.
            self.previous = full;
        }
        self.current.insert(h, e);
    }
}

/// The memoised entry of the position after `auction`, if there is one (never computes).
pub(crate) fn peek(
    table: &Table,
    auction: &Auction,
    implicit_pass: ImplicitPass,
) -> Option<Rc<PrefixEntry>> {
    let tables = addresses(table);
    let complement = implicit_pass == ImplicitPass::Complement;
    let h = hash(&tables, complement, auction);
    MEMO.with(|m| m.borrow_mut().get(h, &tables, complement, auction))
}

/// The entry of the position after `auction`: memoised, or created with `core()` (which must be
/// the position's [`PositionCore`] under `table` and `implicit_pass`).
pub(crate) fn entry(
    table: &Table,
    auction: &Auction,
    implicit_pass: ImplicitPass,
    core: impl FnOnce() -> PositionCore,
) -> Rc<PrefixEntry> {
    let tables = addresses(table);
    let complement = implicit_pass == ImplicitPass::Complement;
    let h = hash(&tables, complement, auction);
    if let Some(e) = MEMO.with(|m| m.borrow_mut().get(h, &tables, complement, auction)) {
        return e;
    }
    let e = Rc::new(PrefixEntry {
        tables,
        complement,
        dealer: auction.dealer(),
        vul: auction.vulnerability(),
        calls: auction.calls().to_vec(),
        _keep: (
            [
                Arc::downgrade(&table.systems[0]),
                Arc::downgrade(&table.systems[1]),
                Arc::downgrade(&table.systems[2]),
                Arc::downgrade(&table.systems[3]),
            ],
            Arc::downgrade(&table.natural),
        ),
        core: core(),
        natural: RefCell::new(None),
    });
    MEMO.with(|m| m.borrow_mut().insert(h, e.clone()));
    e
}
