//! The record of fetches from code a guest store changed before the
//! instruction-modification sequence completed.
//!
//! [PPC-Book2 p:11 s:1.8] after a store into an instruction location,
//! running that location is boundedly undefined until the program runs
//! `dcbst`, `sync`, `icbi`, `isync`; the PPE's L1 instruction cache
//! does not snoop [CBE-Handbook p:145 s:6.1.6.1]. CellGov keeps its eager
//! invalidation and records the fetches that rely on it. `dcbst` and
//! `sync` change no memory in CellGov, since a store is in memory at
//! commit, but a program that leaves them out relies on that.
//!
//! Per 128-byte line [CBE-Handbook p:52 s:2.2] a committed store into a
//! code span marks the line with its writer and step. The tracker then
//! asks for the sequence in its order [PPC-Book2 p:12 s:1.8]: a `dcbst`
//! on the line, a `sync`, and an `icbi` on the line, each by any unit,
//! since `icbi` reaches every PPE [CBE-Handbook p:145 s:6.1.6.1]. A
//! later `isync` makes the line consistent for the unit that ran it. For
//! a unit other than the writer, any context-synchronizing instruction
//! does, `sc` among them [PPC-Book3 p:4 s:1.6.1]. The tracker records
//! the first fetch by each unit from a marked line that is not yet
//! consistent for that unit. A new store to the line starts a new mark
//! that no unit has a record for.
//!
//! A unit hands the runtime its events in program order. The runtime
//! replays them when the batch commits, so the record does not depend on
//! where a batch ends. Units emit events only in traced runs. The record
//! is host-side: it is outside the state hash, and a snapshot restore
//! clears it.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_event::UnitId;
use cellgov_exec::CodeEvent;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_B_OPCD, PPC_NOP};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CACHE_LINE_BYTES;
use cellgov_trace::{TraceRecord, SEQUENCE_DCBST, SEQUENCE_ICBI, SEQUENCE_SYNC};

use crate::runtime::code_writes::read_word;
use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::state::Runtime;

/// The block `icbi` invalidates: one cache line [CBE-Handbook p:52 s:2.2].
const LINE_BYTES: u64 = PPU_CACHE_LINE_BYTES;

/// One fetch from a code line before the instruction-modification
/// sequence completed for that line and the fetching unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsyncedCodeFetch {
    /// The unit that fetched.
    pub unit: UnitId,
    /// The fetched word's address.
    pub pc: u64,
    /// The address space of the line.
    pub space: AddressSpaceId,
    /// The base of the 128-byte line.
    pub line: u64,
    /// The unit whose store last changed the line.
    pub writer: UnitId,
    /// The runtime step that committed that store.
    pub store_step: u64,
    /// The sequence steps that ran after the store, as the
    /// `cellgov_trace::SEQUENCE_*` bits.
    pub sequence: u8,
    /// Whether every store to the line swapped a word between a branch
    /// and `ori 0,0,0` [PPC-Book2 p:13 s:1.8.1].
    pub patch_class: bool,
}

/// One marked line.
#[derive(Debug, Clone)]
struct Line {
    writer: UnitId,
    store_step: u64,
    sequence: u8,
    patch_class: bool,
    invalidated: bool,
    consistent: BTreeSet<UnitId>,
    reported: BTreeSet<UnitId>,
}

/// The tracker the runtime keeps: the marked lines, the events of the
/// step awaiting its commit, and the records so far.
#[derive(Debug, Clone, Default)]
pub(super) struct CodeCoherence {
    lines: BTreeMap<(u32, u64), Line>,
    pending: Option<(UnitId, Vec<CodeEvent>)>,
    records: Vec<UnsyncedCodeFetch>,
}

impl CodeCoherence {
    /// Hold `events` from `unit`'s step until its commit.
    pub(super) fn stash(&mut self, unit: UnitId, events: Vec<CodeEvent>) {
        self.pending = (!events.is_empty()).then_some((unit, events));
    }

    /// The events `unit`'s step left for its commit.
    pub(super) fn take(&mut self, unit: UnitId) -> Vec<CodeEvent> {
        match self.pending.take() {
            Some((owner, events)) if owner == unit => events,
            _ => Vec::new(),
        }
    }

    /// Forget every line, pending event and record.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

/// The base of the line that holds `addr`.
fn line_of(addr: u64) -> u64 {
    addr & !(LINE_BYTES - 1)
}

/// Whether `w` is a word of the patch class: a `b[l][a]` or `ori 0,0,0`.
fn is_patch_word(w: u32) -> bool {
    w == PPC_NOP || w >> 26 == PPC_B_OPCD
}

/// Whether a store of `word` at `addr` swaps one patch-class word for
/// another. The replaced word is the batch's own earlier store to `addr`
/// in `written`, else `before`. The function adds a word store to
/// `written`.
fn store_is_patch(
    written: &mut BTreeMap<u64, u32>,
    addr: u64,
    before: Option<u32>,
    word: Option<u32>,
) -> bool {
    let Some(new) = word else {
        return false;
    };
    let old = written.insert(addr, new).or(before);
    old.is_some_and(is_patch_word) && is_patch_word(new)
}

/// Whether `[start, end)` overlaps a code span.
fn touches_code(spans: &[(u64, u64)], start: u64, end: u64) -> bool {
    let idx = spans.partition_point(|&(_, e)| e <= start);
    spans.get(idx).is_some_and(|&(b, _)| b < end)
}

/// The lines of `[addr, addr + len)` that overlap a code span.
fn code_lines(spans: &[(u64, u64)], addr: u64, len: u8) -> impl Iterator<Item = u64> + '_ {
    let end = addr.saturating_add(u64::from(len));
    let first = line_of(addr);
    let last = line_of(end.saturating_sub(1).max(addr));
    (0..=(last - first) / LINE_BYTES)
        .map(move |i| first + i * LINE_BYTES)
        .filter(move |&line| {
            touches_code(
                spans,
                addr.max(line),
                end.min(line.saturating_add(LINE_BYTES)),
            )
        })
}

impl Runtime {
    /// Every unsynchronized code fetch recorded since the last restore.
    #[must_use]
    pub fn unsynced_code_fetches(&self) -> &[UnsyncedCodeFetch] {
        &self.code_coherence.records
    }

    /// The word each word store in `events` replaced, read from `space`
    /// before the batch commits: `None` for an event that is no word
    /// store into code.
    pub(super) fn code_event_old_words(
        &self,
        space: AddressSpaceId,
        events: &[CodeEvent],
    ) -> Vec<Option<u32>> {
        if events.is_empty() {
            return Vec::new();
        }
        let Ok(memory) = self.space_memory(space) else {
            return Vec::new();
        };
        let spans = memory.code_spans();
        events
            .iter()
            .map(|event| match *event {
                CodeEvent::Store {
                    addr,
                    word: Some(_),
                    ..
                } if touches_code(spans, addr, addr.saturating_add(4)) => read_word(memory, addr),
                _ => None,
            })
            .collect()
    }

    /// Replay `unit`'s committed `events` in program order against the
    /// marked lines of `space`. `old_words` holds what
    /// [`Self::code_event_old_words`] read before the commit.
    pub(super) fn apply_code_events(
        &mut self,
        unit: UnitId,
        space: AddressSpaceId,
        events: &[CodeEvent],
        old_words: &[Option<u32>],
    ) {
        if events.is_empty() {
            return;
        }
        let spans: Vec<(u64, u64)> = self
            .space_memory(space)
            .map(|m| m.code_spans().to_vec())
            .unwrap_or_default();
        let step = self.steps_taken as u64;
        let raw_space = space.raw();
        // The batch's own word stores, so a second store to a word sees
        // the first as the word it replaced.
        let mut written: BTreeMap<u64, u32> = BTreeMap::new();
        for (i, event) in events.iter().enumerate() {
            match *event {
                CodeEvent::Store { addr, len, word } => {
                    let before = old_words.get(i).copied().flatten();
                    let patch = store_is_patch(&mut written, addr, before, word);
                    self.code_coherence.mark_lines(
                        raw_space,
                        code_lines(&spans, addr, len),
                        unit,
                        step,
                        patch,
                    );
                }
                CodeEvent::Dcbst { addr } => {
                    self.code_coherence
                        .add_step(raw_space, line_of(addr), SEQUENCE_DCBST);
                }
                CodeEvent::Sync => self.code_coherence.add_sync(raw_space),
                CodeEvent::Icbi { addr } => {
                    self.code_coherence.invalidate(raw_space, line_of(addr))
                }
                CodeEvent::Isync => self.code_coherence.synchronize(raw_space, unit, true),
                CodeEvent::ContextSync => self.code_coherence.synchronize(raw_space, unit, false),
                CodeEvent::Fetch { pc } => self.record_unsynced_code_fetch(space, unit, pc),
            }
        }
    }

    /// Record a fetch at `pc` by `unit`, if its line owes one.
    fn record_unsynced_code_fetch(&mut self, space: AddressSpaceId, unit: UnitId, pc: u64) {
        let Some(record) = self.code_coherence.fetch(space, unit, pc) else {
            return;
        };
        self.trace.record(&TraceRecord::UnsyncedCodeFetch {
            unit: record.unit,
            pc: record.pc,
            space: record.space.raw(),
            line: record.line,
            writer: record.writer,
            store_step: record.store_step,
            sequence: record.sequence,
            patch_class: record.patch_class,
        });
        self.code_coherence.records.push(record);
    }
}

impl CodeCoherence {
    /// A store into code changed `line`: mark it afresh, or extend the
    /// mark if no `icbi` invalidated the line after the last store.
    fn mark(&mut self, space: u32, line: u64, writer: UnitId, store_step: u64, patch: bool) {
        let patch_class = match self.lines.get(&(space, line)) {
            Some(l) if !l.invalidated => l.patch_class && patch,
            _ => patch,
        };
        self.lines.insert(
            (space, line),
            Line {
                writer,
                store_step,
                sequence: 0,
                patch_class,
                invalidated: false,
                consistent: BTreeSet::new(),
                reported: BTreeSet::new(),
            },
        );
    }

    /// [`Self::mark`] each of `lines`.
    fn mark_lines(
        &mut self,
        space: u32,
        lines: impl Iterator<Item = u64>,
        writer: UnitId,
        store_step: u64,
        patch: bool,
    ) {
        for line in lines {
            self.mark(space, line, writer, store_step, patch);
        }
    }

    fn add_step(&mut self, space: u32, line: u64, bit: u8) {
        if let Some(l) = self.lines.get_mut(&(space, line)) {
            l.sequence |= bit;
        }
    }

    /// A `sync` orders the `dcbst`s before it, so it counts on the lines
    /// a `dcbst` copied.
    fn add_sync(&mut self, space: u32) {
        for (_, l) in self.lines.range_mut((space, 0)..=(space, u64::MAX)) {
            if l.sequence & SEQUENCE_DCBST != 0 {
                l.sequence |= SEQUENCE_SYNC;
            }
        }
    }

    /// An `icbi` invalidates the line; after `dcbst` and `sync` it leaves
    /// the line waiting only for a context synchronization.
    fn invalidate(&mut self, space: u32, line: u64) {
        if let Some(l) = self.lines.get_mut(&(space, line)) {
            l.invalidated |= l.sequence & SEQUENCE_SYNC != 0;
            l.sequence |= SEQUENCE_ICBI;
        }
    }

    /// A context-synchronizing instruction by `unit` makes every
    /// invalidated line consistent for it. One other than `isync` does so
    /// only on the lines another unit wrote [PPC-Book2 p:12 s:1.8].
    fn synchronize(&mut self, space: u32, unit: UnitId, isync: bool) {
        for (_, l) in self.lines.range_mut((space, 0)..=(space, u64::MAX)) {
            if l.invalidated && (isync || l.writer != unit) {
                l.consistent.insert(unit);
            }
        }
    }

    /// The record a fetch at `pc` by `unit` makes, if a store marked its
    /// line and the line is neither consistent for nor reported to `unit`.
    fn fetch(&mut self, space: AddressSpaceId, unit: UnitId, pc: u64) -> Option<UnsyncedCodeFetch> {
        let line = line_of(pc);
        let l = self.lines.get_mut(&(space.raw(), line))?;
        if l.consistent.contains(&unit) || !l.reported.insert(unit) {
            return None;
        }
        Some(UnsyncedCodeFetch {
            unit,
            pc,
            space,
            line,
            writer: l.writer,
            store_step: l.store_step,
            sequence: l.sequence,
            patch_class: l.patch_class,
        })
    }
}

#[cfg(test)]
#[path = "tests/code_coherence_tests.rs"]
mod tests;
