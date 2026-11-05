//! The committed guest stores that land in declared code.
//!
//! A unit's store whose range overlaps a code span of its address space
//! rewrites an instruction the process may run. The runtime keeps a
//! host-side record of each one: the writer, the step, the 128-byte line
//! and the first code word the store covers, before and after. The
//! record is outside the state hash and a snapshot leaves it alone, the
//! way the other host-side witnesses are.

use std::collections::BTreeMap;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CACHE_LINE_BYTES;

use crate::runtime::commit_step::committed_store_range;
use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::state::Runtime;

/// How many stores the record keeps in full; later ones still count.
const CODE_WRITE_DETAIL_LIMIT: usize = 64;

/// One committed guest store into declared code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeWrite {
    /// The unit whose batch committed the store.
    pub unit: UnitId,
    /// The address space the store landed in.
    pub space: AddressSpaceId,
    /// The runtime step that committed it.
    pub step: u64,
    /// The address of the first code word the store covers.
    pub word: u64,
    /// That word before the store's batch commits.
    pub before: u32,
    /// That word after the store's batch commits, so it includes any
    /// later store of the same batch to the word.
    pub after: u32,
}

impl CodeWrite {
    /// The base of the 128-byte cache line that holds the word
    /// [CBE-Handbook p:52 s:2.2].
    #[must_use]
    pub fn line(&self) -> u64 {
        self.word & !(PPU_CACHE_LINE_BYTES - 1)
    }
}

/// Every committed guest store into declared code over a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeWriteWitness {
    stores: u64,
    lines: BTreeMap<(u32, u64), u64>,
    detail: Vec<CodeWrite>,
}

impl CodeWriteWitness {
    /// Stores counted, including those past the detail limit.
    #[must_use]
    pub fn stores(&self) -> u64 {
        self.stores
    }

    /// The stores per (space, line base), in address order.
    pub fn lines(&self) -> impl Iterator<Item = (AddressSpaceId, u64, u64)> + '_ {
        self.lines
            .iter()
            .map(|(&(space, line), &n)| (AddressSpaceId::new(space), line, n))
    }

    /// The stores kept in full, in commit order.
    #[must_use]
    pub fn detail(&self) -> &[CodeWrite] {
        &self.detail
    }

    /// Count `write` under its line, and keep it while the detail has
    /// room.
    pub fn record(&mut self, write: CodeWrite) {
        self.stores = self.stores.wrapping_add(1);
        let hits = self
            .lines
            .entry((write.space.raw(), write.line()))
            .or_insert(0);
        *hits = hits.wrapping_add(1);
        if self.detail.len() < CODE_WRITE_DETAIL_LIMIT {
            self.detail.push(write);
        }
    }
}

/// The code word a store covers, read before the store's batch commits.
#[derive(Debug, Clone, Copy)]
pub(super) struct PendingCodeWrite {
    word: u64,
    before: u32,
}

/// The first code word that `range` covers, or `None` when the range
/// touches no code span.
fn first_code_word(spans: &[(u64, u64)], range: ByteRange) -> Option<u64> {
    let start = range.start().raw();
    let end = start.checked_add(range.length())?;
    // The spans are sorted and disjoint: the first one ending past
    // `start` is the only candidate for the lowest overlap.
    let idx = spans.partition_point(|&(_, e)| e <= start);
    let &(b, e) = spans.get(idx)?;
    (b < end && start < e).then(|| start.max(b) & !3)
}

fn read_word(memory: &GuestMemory, addr: u64) -> Option<u32> {
    let range = ByteRange::new(GuestAddr::new(addr), 4)?;
    memory
        .read(range)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

impl Runtime {
    /// The record of committed guest stores into declared code.
    #[must_use]
    pub fn code_writes(&self) -> &CodeWriteWitness {
        &self.code_writes
    }

    /// Read the code words a batch's stores cover, before the batch
    /// commits. The result is empty, with no allocation, when no store
    /// touches code.
    pub(super) fn code_writes_before(
        &self,
        space: AddressSpaceId,
        effects: &[Effect],
    ) -> Vec<PendingCodeWrite> {
        let Ok(memory) = self.space_memory(space) else {
            return Vec::new();
        };
        let spans = memory.code_spans();
        if spans.is_empty() {
            return Vec::new();
        }
        effects
            .iter()
            .filter_map(committed_store_range)
            .filter_map(|range| first_code_word(spans, range))
            .filter_map(|word| {
                read_word(memory, word).map(|before| PendingCodeWrite { word, before })
            })
            .collect()
    }

    /// Record the stores that [`Self::code_writes_before`] found, after
    /// their batch commits.
    pub(super) fn record_code_writes(
        &mut self,
        unit: UnitId,
        space: AddressSpaceId,
        pending: &[PendingCodeWrite],
    ) {
        let step = self.steps_taken as u64;
        for p in pending {
            let after = self
                .space_memory(space)
                .ok()
                .and_then(|m| read_word(m, p.word))
                .unwrap_or(p.before);
            self.code_writes.record(CodeWrite {
                unit,
                space,
                step,
                word: p.word,
                before: p.before,
                after,
            });
        }
    }
}

#[cfg(test)]
#[path = "tests/code_writes_tests.rs"]
mod tests;
