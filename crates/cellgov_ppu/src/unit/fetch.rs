//! The text runs a block fetches, and their publication at the block
//! boundary.

use super::ppu_unit::PpuExecutionUnit;
use crate::mmu::Access;
use cellgov_effects::Effect;

/// Runs one block reports apart before it collapses them into one
/// span.
pub(super) const FETCH_RUNS_MAX: usize = 8;

/// [`PpuExecutionUnit::fetch_end`] when no run is open.
///
/// The value is odd and every branch form writes a word-aligned
/// target, so no pc reaches it from an aligned entry.
pub(super) const NO_FETCH_RUN: u64 = u64::MAX;

impl PpuExecutionUnit {
    /// Publish what the block did to committed memory: the stores it
    /// buffered, and the text it fetched.
    ///
    /// Both reach the effect list at the block boundary rather than per
    /// instruction, so the two orders a write to the text region and a
    /// fetch of it can take are held apart without a packet per fetch.
    ///
    /// A thread that translates publishes both at real addresses.
    pub(super) fn close_block(&mut self, effects: &mut Vec<Effect>) {
        let translates = self.translates();
        self.store_buf.flush(effects, self.id);
        if translates {
            self.real_stores(effects);
        }
        if self.fetch_end != NO_FETCH_RUN {
            self.fetch_runs.push((self.fetch_start, self.fetch_end));
            self.fetch_end = NO_FETCH_RUN;
        }
        let fetches_from = effects.len();
        for (start, end) in self.fetch_runs.drain(..) {
            let range =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(start), end - start)
                    .expect("a run ends above its start, so its end is the u64 the range needs");
            effects.push(Effect::SharedReadIntent {
                range,
                source: self.id,
            });
        }
        if translates {
            self.real_effects(effects, fetches_from, Access::Fetch);
            self.publish_table_reads(effects);
        }
        if self.state.clock_read {
            effects.push(Effect::ClockRead { source: self.id });
        }
    }

    /// Extend the run in flight, or start another.
    ///
    /// `pc` names a word the unit fetched, so `pc + 4` cannot overflow
    /// here or in [`Self::start_fetch_run`]. On a thread that does not
    /// translate, `pc + 4` is at most the memory's length. On a thread
    /// that translates, `pc` is an effective address, and the fetch
    /// refuses the last word of the effective space, so `pc + 4` is at
    /// most `u64::MAX - 3`. A pc that names no word faults in the
    /// fetch, before any run records it.
    #[inline(always)]
    pub(super) fn note_fetch(&mut self, pc: u64) {
        // Every branch form writes a target with the low two bits
        // clear, so the sentinel's odd address names no fetch. The
        // assertion pins that rather than trusting it.
        debug_assert_ne!(pc, NO_FETCH_RUN, "a fetch at the no-run sentinel address");
        if self.fetch_end == pc {
            self.fetch_end = pc + 4;
        } else {
            self.start_fetch_run(pc);
        }
    }

    /// Drop the runs the block recorded without publishing them.
    ///
    /// A discarded batch retired nothing, so the text it fetched
    /// reaches no effect list. An open run left behind would instead
    /// publish at the next block's boundary, naming addresses that
    /// block never fetched.
    pub(super) fn drop_fetch_runs(&mut self) {
        self.fetch_end = NO_FETCH_RUN;
        self.fetch_runs.clear();
    }

    /// Close the run in flight and open one at `pc`.
    ///
    /// Cold: a block reaches it once, plus once per branch it takes.
    /// Past [`FETCH_RUNS_MAX`] runs it stops tracking them apart and
    /// keeps one span, which is what a loop body would otherwise cost
    /// a run per iteration.
    ///
    /// A translating thread keeps one span only when one fetch
    /// translation holds all of it. A wider span names effective
    /// addresses the block did not fetch, which have no real address,
    /// and every address an effect carries is real. Otherwise its runs
    /// stay apart and the block keeps each run once, so a loop body
    /// costs no run per iteration.
    #[cold]
    fn start_fetch_run(&mut self, pc: u64) {
        if self.fetch_end == NO_FETCH_RUN {
            self.fetch_start = pc;
            self.fetch_end = pc + 4;
            return;
        }
        let run = (self.fetch_start, self.fetch_end);
        if self.fetch_runs.len() >= FETCH_RUNS_MAX {
            let (start, end) = self
                .fetch_runs
                .iter()
                .fold((run.0.min(pc), run.1.max(pc + 4)), |(s, e), &(rs, re)| {
                    (s.min(rs), e.max(re))
                });
            if self.one_fetch_span(start, end) {
                self.fetch_runs.clear();
                self.fetch_start = start;
                self.fetch_end = end;
                return;
            }
            if self.fetch_runs.contains(&run) {
                self.fetch_start = pc;
                self.fetch_end = pc + 4;
                return;
            }
        }
        self.fetch_runs.push(run);
        self.fetch_start = pc;
        self.fetch_end = pc + 4;
    }

    /// Whether the block can report `[start, end)` as one span: the
    /// thread does not translate, or one fetch translation holds the
    /// whole span.
    fn one_fetch_span(&self, start: u64, end: u64) -> bool {
        !self.translates()
            || self
                .translations
                .lookup(Access::Fetch, start)
                .is_some_and(|t| t.real(end - 1).is_some())
    }
}
