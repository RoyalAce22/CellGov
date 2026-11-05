//! The PPU unit's state, its constructor, and its accessors.

use super::fetch::NO_FETCH_RUN;
use crate::observation::PpuArchitecturalState;
use crate::store_buffer::StoreBuffer;
use crate::tap::PpuTap;
use crate::{shadow, state};
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

/// One PPU thread's architected state and the unit's run state.
///
/// The differential snapshot, the observation and the fingerprint are
/// projections of [`Self::state`]. [`PpuExecutionUnit::restore`] is the
/// exact inverse of the unit's snapshot.
///
/// [CBE-Handbook p:297 s:10.2] for each thread, the PPE has duplicate copies of all architected state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuSnapshot {
    /// Registers, PC, time base and the reservation.
    ///
    /// [PPC-Book1 p:18 s:2.3.1] CR is 32 bits in eight 4-bit fields.
    /// [PPC-Book1 p:19 s:2.3] LR and CTR are 64-bit branch registers.
    /// [PPC-Book2 p:29 s:Chapter 4. Time Base] TB is a 64-bit unsigned counter that the hardware increments periodically.
    pub state: PpuArchitecturalState,
    /// The status the unit resumes with.
    pub status: UnitStatus,
    /// The PC a debug break waits for.
    pub break_pc: Option<u64>,
    /// The hits at `break_pc` still to pass before the break fires.
    pub break_skip: u32,
    /// The issue model, which times the next instruction.
    pub(super) timing: Box<crate::timing::PpeTiming>,
}

/// PPU `ExecutionUnit`: owns architectural state, fetches and executes
/// instructions, emits `Effect`s for stores and syscalls.
#[derive(Clone)]
pub struct PpuExecutionUnit {
    pub(super) id: UnitId,
    pub(super) state: state::PpuState,
    pub(super) status: UnitStatus,
    /// Fires `FAULT_DEBUG_BREAK` after `break_skip` prior hits at this PC.
    pub(super) break_pc: Option<u64>,
    pub(super) break_skip: u32,
    pub(super) per_step_hashes: Vec<(u64, u64)>,
    pub(super) full_state_window: Option<(u64, u64)>,
    /// Increments on successful retirement only.
    pub(super) retirement_counter: u64,
    pub(super) per_step_full_states: Vec<(u64, u64, cellgov_exec::PpuFingerprint)>,
    /// Barrier instructions retired since the last drain.
    ///
    /// The unit records a barrier only while the context asks for
    /// per-step trace data. A faulted batch truncates this list to its
    /// length at batch entry.
    pub(super) barriers: Vec<cellgov_exec::RetiredBarrier>,
    pub(super) instruction_shadow: Option<shadow::PredecodedShadow>,
    pub(super) shadow_hits: u64,
    pub(super) shadow_misses: u64,
    /// Start of the run of text the block is fetching from now.
    ///
    /// A fetch reads the text region whether or not the shadow answers
    /// it: a committed write there drives `invalidate_code` over the
    /// slot, so the next fetch takes the new bytes. Fetch is the
    /// highest-frequency read a boot has, so a straight run costs one
    /// compare and one add per instruction here and one read intent at
    /// the block boundary.
    pub(super) fetch_start: u64,
    /// End of that run, or [`NO_FETCH_RUN`] when the block has none.
    ///
    /// A fetch that continues the run is one compare against this and
    /// one store back, which is what keeps the highest-frequency read
    /// in a boot off the profile.
    pub(super) fetch_end: u64,
    /// Runs this block finished before the one `fetch_start` and
    /// `fetch_end` hold.
    ///
    /// A branch closes a run and opens another. Past
    /// [`FETCH_RUNS_MAX`](super::fetch::FETCH_RUNS_MAX) the block stops
    /// tracking them apart and reports one span covering every address
    /// it fetched, which is what a loop body would otherwise cost a run
    /// per iteration.
    pub(super) fetch_runs: Vec<(u64, u64)>,
    pub(super) store_buf: StoreBuffer,
    /// The issue model that prices each retired instruction in core
    /// cycles.
    pub(super) timing: Box<crate::timing::PpeTiming>,
    pub(super) profile_mode: bool,
    pub(super) profile_insns: std::collections::BTreeMap<&'static str, u64>,
    pub(super) profile_pairs: std::collections::BTreeMap<(&'static str, &'static str), u64>,
    pub(super) profile_prev: Option<&'static str>,
    /// `Clone` shares the tap, so a cloned unit reports to the same
    /// observer. A runtime snapshot holds such a clone.
    pub(super) tap: Option<std::rc::Rc<dyn PpuTap>>,
}

impl PpuExecutionUnit {
    /// Fresh PPU unit with the given id and zeroed architectural state.
    pub fn new(id: UnitId) -> Self {
        Self {
            id,
            state: state::PpuState::new(),
            status: UnitStatus::Runnable,
            break_pc: None,
            break_skip: 0,
            per_step_hashes: Vec::new(),
            full_state_window: None,
            retirement_counter: 0,
            per_step_full_states: Vec::new(),
            barriers: Vec::new(),
            instruction_shadow: None,
            shadow_hits: 0,
            shadow_misses: 0,
            fetch_start: 0,
            fetch_end: NO_FETCH_RUN,
            fetch_runs: Vec::new(),
            store_buf: StoreBuffer::new(),
            timing: Box::default(),
            profile_mode: false,
            profile_insns: std::collections::BTreeMap::new(),
            profile_pairs: std::collections::BTreeMap::new(),
            profile_prev: None,
            tap: None,
        }
    }

    /// Report every dispatched instruction to `tap`.
    pub fn set_tap(&mut self, tap: std::rc::Rc<dyn PpuTap>) {
        self.tap = Some(tap);
    }

    /// Set the inclusive `[lo, hi]` retirement-index window for full-state capture.
    pub fn set_full_state_window(&mut self, window: Option<(u64, u64)>) {
        self.full_state_window = window;
    }

    /// Returns the current full-state-capture retirement-index window.
    pub fn full_state_window(&self) -> Option<(u64, u64)> {
        self.full_state_window
    }

    /// Skip `skip` hits at `pc`, then fault on the next.
    pub fn set_break_pc(&mut self, pc: u64, skip: u32) {
        self.break_pc = Some(pc);
        self.break_skip = skip;
    }

    /// Toggle per-instruction profile accumulation.
    pub fn set_profile_mode(&mut self, on: bool) {
        self.profile_mode = on;
    }

    /// Returns the accumulated per-instruction execution counts.
    pub fn profile_insns(&self) -> &std::collections::BTreeMap<&'static str, u64> {
        &self.profile_insns
    }

    /// Returns the accumulated adjacent-instruction-pair execution counts.
    pub fn profile_pairs(&self) -> &std::collections::BTreeMap<(&'static str, &'static str), u64> {
        &self.profile_pairs
    }

    /// Mutable access to architectural state.
    pub fn state_mut(&mut self) -> &mut state::PpuState {
        &mut self.state
    }

    /// Shared access to architectural state.
    pub fn state(&self) -> &state::PpuState {
        &self.state
    }

    /// Install the predecoded shadow. Caller must build it after all
    /// boot-time code writes (ELF/PRX load, HLE stub planting) and
    /// before the step loop begins; stale slots re-decode on every fetch.
    pub fn set_instruction_shadow(&mut self, shadow: shadow::PredecodedShadow) {
        self.instruction_shadow = Some(shadow);
    }

    /// Returns `(hits, misses)`; high miss ratios lose the O(1) fast path.
    pub fn shadow_stats(&self) -> (u64, u64) {
        (self.shadow_hits, self.shadow_misses)
    }

    /// Instructions the unit retired since it was built. A faulted
    /// batch retires none.
    pub fn retired_instructions(&self) -> u64 {
        self.retirement_counter
    }

    /// Put the unit back in the context `snapshot` holds. The registers
    /// go through their setters, so the state hash is current after the
    /// restore. The unit keeps its id, its instruments and its trace
    /// settings, and drops the trace output it has not drained.
    pub fn restore(&mut self, snapshot: PpuSnapshot) {
        let PpuSnapshot {
            state,
            status,
            break_pc,
            break_skip,
            timing,
        } = snapshot;
        // Every field is named, so a new one fails to compile here until
        // the restore sets it, clears it or says why the unit keeps it.
        let PpuExecutionUnit {
            // The unit's identity.
            id: _,
            state: unit_state,
            status: unit_status,
            break_pc: unit_break_pc,
            break_skip: unit_break_skip,
            // Trace output the runtime has not drained.
            per_step_hashes,
            per_step_full_states,
            barriers,
            // Trace instruments and settings.
            retirement_counter: _,
            full_state_window: _,
            // A cache of guest memory that `invalidate_code` keeps current.
            instruction_shadow: _,
            shadow_hits: _,
            shadow_misses: _,
            // Batch-local: each batch entry clears them.
            fetch_start: _,
            fetch_end: _,
            fetch_runs: _,
            store_buf: _,
            timing: unit_timing,
            // Profile instruments and the observer.
            profile_mode: _,
            profile_insns: _,
            profile_pairs: _,
            profile_prev: _,
            tap: _,
        } = self;
        state.apply(unit_state);
        *unit_status = status;
        *unit_break_pc = break_pc;
        *unit_break_skip = break_skip;
        *unit_timing = timing;
        per_step_hashes.clear();
        per_step_full_states.clear();
        barriers.clear();
    }
}
