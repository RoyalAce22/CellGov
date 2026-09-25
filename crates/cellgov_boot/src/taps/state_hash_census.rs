//! A census of the PPU states a run passes through, and of how many of
//! them share a state hash.
//!
//! A state is the field set of [`PpuState::fingerprint`], which excludes
//! the PC. The census counts collisions for two functions over the same
//! states:
//!
//! - [`PpuState::state_hash`], the hash the trace records;
//! - [`multilinear::hash`], the multilinear construction.
//!
//! Two records count as one state when two further multilinear hashes,
//! with independent keys, agree on both. Two distinct states agree on
//! both with a probability of at most 2^-128.
//!
//! The census reads each state at dispatch, before its instruction
//! runs. A dispatch is not a retirement (see [`PpuTap::dispatch`]), so
//! the states it counts are not exactly the states the trace records:
//!
//! - It counts the states of a batch that a fault rolls back. The
//!   rollback erases those states from the trace.
//! - It does not see the end state of a batch when something changes
//!   that state before the next dispatch (a syscall return, a register
//!   write, a cleared reservation). It also does not see the state each
//!   unit ends in.
//! - A store that retries on a full store buffer dispatches one state
//!   two times. The second dispatch adds no state.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cellgov_event::UnitId;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::multilinear::{self, KEY_COUNT, LANE_COUNT};
use cellgov_ppu::state::PpuState;
use cellgov_ppu::PpuTap;

use super::DebugTaps;

/// The SplitMix64 seeds of the two identity key sets. Each differs from
/// [`multilinear::KEY_SEED`], so neither set is the construction's own.
const IDENTITY_SEEDS: [u64; 2] = [1, 2];

/// One dispatched state.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Record {
    identity: (u64, u64),
    state_hash: u64,
    multilinear: u64,
}

/// A PPU observer that records every dispatched state for a
/// [`CensusReport`].
///
/// It holds 32 bytes per dispatch until it drops.
pub struct StateHashCensus {
    identity_keys: [[u128; KEY_COUNT]; 2],
    sample_every: u64,
    dispatches: Cell<u64>,
    records: RefCell<Vec<Record>>,
    samples: RefCell<Vec<(u64, [u64; LANE_COUNT])>>,
}

/// What a [`StateHashCensus`] counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CensusReport {
    /// Dispatches the census saw.
    pub dispatches: u64,
    /// States among them, each counted once.
    pub distinct_states: u64,
    /// Distinct states less distinct [`PpuState::state_hash`] values.
    ///
    /// A group of k states with one hash counts k - 1.
    pub state_hash_collisions: u64,
    /// Distinct states less distinct [`multilinear::hash`] values,
    /// counted as for [`Self::state_hash_collisions`].
    pub multilinear_collisions: u64,
    /// Records after the first under one identity, each with a different
    /// pair of hashes.
    ///
    /// A nonzero count means the identity hashes collided, and the
    /// other counts are not reliable.
    pub identity_conflicts: u64,
    /// The lanes at every dispatch whose index is a multiple of the
    /// sample interval, with that index.
    pub samples: Vec<(u64, [u64; LANE_COUNT])>,
}

impl StateHashCensus {
    /// A census that also keeps the lanes of every `sample_every`-th
    /// dispatch, starting at the first. A `sample_every` of 0 keeps
    /// none.
    pub fn new(sample_every: u64) -> Self {
        Self {
            identity_keys: IDENTITY_SEEDS.map(multilinear::derive_keys),
            sample_every,
            dispatches: Cell::new(0),
            records: RefCell::new(Vec::new()),
            samples: RefCell::new(Vec::new()),
        }
    }

    /// Count the states recorded so far.
    ///
    /// It sorts and dedups the records in place, so the count needs no
    /// second copy of them.
    pub fn report(&self) -> CensusReport {
        let mut records = self.records.borrow_mut();
        records.sort_unstable();
        records.dedup();
        let mut identity_conflicts = 0;
        let mut state_hashes = Vec::with_capacity(records.len());
        let mut multilinear_hashes = Vec::with_capacity(records.len());
        let mut last = None;
        for r in records.iter() {
            if last == Some(r.identity) {
                identity_conflicts += 1;
                continue;
            }
            last = Some(r.identity);
            state_hashes.push(r.state_hash);
            multilinear_hashes.push(r.multilinear);
        }
        CensusReport {
            dispatches: self.dispatches.get(),
            distinct_states: state_hashes.len() as u64,
            state_hash_collisions: collisions(state_hashes),
            multilinear_collisions: collisions(multilinear_hashes),
            identity_conflicts,
            samples: self.samples.borrow().clone(),
        }
    }
}

/// The length of `v` less the count of distinct values in it.
fn collisions(mut v: Vec<u64>) -> u64 {
    let n = v.len();
    v.sort_unstable();
    v.dedup();
    (n - v.len()) as u64
}

impl PpuTap for StateHashCensus {
    fn dispatch(&self, _unit: UnitId, _insn: &PpuInstruction, state: &PpuState) {
        let index = self.dispatches.get();
        self.dispatches.set(index + 1);
        let lanes = multilinear::lanes(&state.fingerprint());
        let [a, b] = &self.identity_keys;
        self.records.borrow_mut().push(Record {
            identity: (
                multilinear::finish(multilinear::accumulate_with(a, &lanes)),
                multilinear::finish(multilinear::accumulate_with(b, &lanes)),
            ),
            state_hash: state.state_hash(),
            multilinear: multilinear::hash(&lanes),
        });
        if self.sample_every != 0 && index.is_multiple_of(self.sample_every) {
            self.samples.borrow_mut().push((index, lanes));
        }
    }
}

/// Two PPU observers that see each dispatch in turn.
struct PpuTapPair(Rc<dyn PpuTap>, Rc<dyn PpuTap>);

impl PpuTap for PpuTapPair {
    fn dispatch(&self, unit: UnitId, insn: &PpuInstruction, state: &PpuState) {
        self.0.dispatch(unit, insn, state);
        self.1.dispatch(unit, insn, state);
    }
}

/// The observers of `inner`, plus one more PPU observer.
pub struct WithPpuTap {
    inner: Rc<dyn DebugTaps>,
    ppu: Rc<dyn PpuTap>,
}

impl WithPpuTap {
    /// `inner`'s observers, with `extra` beside its PPU observer.
    pub fn new(inner: Rc<dyn DebugTaps>, extra: Rc<dyn PpuTap>) -> Self {
        let ppu = match inner.ppu() {
            Some(first) => Rc::new(PpuTapPair(first, extra)) as Rc<dyn PpuTap>,
            None => extra,
        };
        Self { inner, ppu }
    }
}

impl DebugTaps for WithPpuTap {
    fn ppu(&self) -> Option<Rc<dyn PpuTap>> {
        Some(Rc::clone(&self.ppu))
    }

    fn runtime(&self) -> Option<Box<dyn cellgov_core::RuntimeTap>> {
        self.inner.runtime()
    }

    fn firmware_bound(
        &self,
        space: u32,
        exports: &std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>>,
        mem: &cellgov_mem::GuestMemory,
    ) {
        self.inner.firmware_bound(space, exports, mem);
    }
}

#[cfg(test)]
#[path = "tests/state_hash_census_tests.rs"]
mod tests;
