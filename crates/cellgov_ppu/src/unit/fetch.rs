//! The text runs a block fetches, and their publication at the block
//! boundary.

use super::ppu_unit::PpuExecutionUnit;
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
    pub(super) fn close_block(&mut self, effects: &mut Vec<Effect>) {
        self.store_buf.flush(effects, self.id);
        if self.fetch_end != NO_FETCH_RUN {
            self.fetch_runs.push((self.fetch_start, self.fetch_end));
            self.fetch_end = NO_FETCH_RUN;
        }
        for (start, end) in self.fetch_runs.drain(..) {
            let range =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(start), end - start)
                    .expect("a run ends above its start, so its end is the u64 the range needs");
            effects.push(Effect::SharedReadIntent {
                range,
                source: self.id,
            });
        }
        if self.state.clock_read {
            effects.push(Effect::ClockRead { source: self.id });
        }
    }

    /// Extend the run in flight, or start another.
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
    #[cold]
    fn start_fetch_run(&mut self, pc: u64) {
        if self.fetch_end == NO_FETCH_RUN {
            self.fetch_start = pc;
            self.fetch_end = pc + 4;
            return;
        }
        if self.fetch_runs.len() >= FETCH_RUNS_MAX {
            let mut start = self.fetch_start.min(pc);
            let mut end = self.fetch_end.max(pc + 4);
            for (s, e) in self.fetch_runs.drain(..) {
                start = start.min(s);
                end = end.max(e);
            }
            self.fetch_start = start;
            self.fetch_end = end;
            return;
        }
        self.fetch_runs.push((self.fetch_start, self.fetch_end));
        self.fetch_start = pc;
        self.fetch_end = pc + 4;
    }
}
