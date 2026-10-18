//! The SPU unit's state, its constructor, and its accessors.

use crate::state;
use cellgov_event::UnitId;
use cellgov_exec::{ChannelStall, UnitStatus};

/// The SPU's whole context: its architected state and the unit's run
/// state around it. Replay, instruction comparison, the local-memory
/// hash and a saved context all read this one type, and
/// [`SpuExecutionUnit::restore`] is its exact inverse.
///
/// [CBEA p:241 s:17] an implementation supports a full save and restore of an SPE context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuSnapshot {
    /// Registers, local store, PC, LSLR, FPSCR, the stopped state, IE
    /// and SRR0, the signal registers, every channel's data and count,
    /// the event registers and the reservation.
    pub state: state::SpuState,
    /// Whether the unit runs, waits, stopped or faulted.
    pub status: UnitStatus,
    /// The channel access a parked unit waits on.
    pub stall: Option<ChannelStall>,
    /// The issue state the next instruction's cycle depends on.
    pub pipeline: crate::timing::SpuPipeline,
}

impl SpuSnapshot {
    /// The comparison view of the architected state.
    pub fn observable(&self) -> state::SpuObservableSnapshot {
        state::SpuObservableSnapshot::capture(&self.state)
    }

    /// The hash of the private memory the runtime's observable hash folds
    /// in: the local store, the signal-notification registers and the
    /// outbound mailboxes.
    pub fn local_memory_hash(&self) -> u64 {
        local_memory_hash(&self.state)
    }
}

/// FNV-1a over the SPU state outside committed memory that another
/// processor writes or reads:
///
/// - the local-store bytes
/// - the two signal-notification registers
/// - the two outbound mailboxes
///
/// The problem-state operations change a signal register or a mailbox
/// outside any effect. Two schedules can thus leave the local store
/// equal and a signal register or a mailbox different.
pub(super) fn local_memory_hash(state: &state::SpuState) -> u64 {
    let mut hasher = cellgov_mem::Fnv1aHasher::new();
    hasher.write(&state.ls);
    for signal in &state.signals {
        hasher.write(&signal.word.to_le_bytes());
        hasher.write(&[u8::from(signal.pending)]);
    }
    for message in [state.channels.out_mbox, state.channels.out_intr_mbox] {
        match message {
            Some(word) => {
                hasher.write(&[1]);
                hasher.write(&word.to_le_bytes());
            }
            None => hasher.write(&[0]),
        }
    }
    hasher.finish()
}

/// A Synergistic Processing Unit execution unit.
#[derive(Clone)]
pub struct SpuExecutionUnit {
    pub(super) id: UnitId,
    pub(super) state: state::SpuState,
    pub(super) status: UnitStatus,
    /// The channel access the last step stalled on; cleared at each
    /// step entry, since the woken step runs the access again.
    pub(super) stall: Option<ChannelStall>,
    /// The issue state: the cycle the next instruction issues in.
    pub(super) pipeline: crate::timing::SpuPipeline,
    /// Barrier instructions retired since the last drain.
    ///
    /// The unit records a barrier only while the context asks for
    /// per-step trace data.
    pub(super) barriers: Vec<cellgov_exec::RetiredBarrier>,
    /// Instructions the unit has retired: the step each state record
    /// carries.
    pub(super) retirement_counter: u64,
    /// `[lo, hi]` of the retirement steps whose full state the unit
    /// records, or `None` for none.
    pub(super) full_state_window: Option<(u64, u64)>,
    /// `(step, pc, state_hash)` per instruction retired since the last
    /// drain, recorded only while the context asks for per-step trace
    /// data.
    pub(super) per_step_hashes: Vec<(u64, u64, u64)>,
    /// `(step, pc, fingerprint)` per instruction retired inside the
    /// window since the last drain.
    pub(super) per_step_full_states: Vec<(u64, u64, cellgov_exec::SpuFingerprint)>,
}

impl SpuExecutionUnit {
    /// Construct a runnable SPU with zeroed architectural state.
    pub fn new(id: UnitId) -> Self {
        Self {
            id,
            state: state::SpuState::new(),
            status: UnitStatus::Runnable,
            stall: None,
            pipeline: crate::timing::SpuPipeline::new(),
            barriers: Vec::new(),
            retirement_counter: 0,
            full_state_window: None,
            per_step_hashes: Vec::new(),
            per_step_full_states: Vec::new(),
        }
    }

    /// Record the full state of each instruction whose retirement step
    /// falls in `[lo, hi]`, or of none for `None`.
    pub fn set_full_state_window(&mut self, window: Option<(u64, u64)>) {
        self.full_state_window = window;
    }

    /// The window [`Self::set_full_state_window`] set.
    pub fn full_state_window(&self) -> Option<(u64, u64)> {
        self.full_state_window
    }

    /// Record the instruction at `pc` as retired: its state hash while
    /// the context asks for per-step trace data, its full state inside
    /// the window, and one more on the retirement counter.
    pub(super) fn retire(&mut self, pc: u64, trace_per_step: bool) {
        let step = self.retirement_counter;
        if trace_per_step {
            self.per_step_hashes
                .push((step, pc, self.state.state_hash()));
        }
        if self
            .full_state_window
            .is_some_and(|(lo, hi)| (lo..=hi).contains(&step))
        {
            self.per_step_full_states
                .push((step, pc, self.state.fingerprint()));
        }
        self.retirement_counter += 1;
    }

    /// Mutable access to architectural state.
    pub fn state_mut(&mut self) -> &mut state::SpuState {
        &mut self.state
    }

    /// Read access to architectural state.
    pub fn state(&self) -> &state::SpuState {
        &self.state
    }

    /// Put the unit back in the context `snapshot` holds. The unit keeps
    /// its id, its retirement counter and its full-state window, and
    /// drops the barriers and state records it has not drained, which
    /// are trace output, not state.
    ///
    /// [CBEA p:241 s:17] a context restore returns the SPE to the saved state.
    pub fn restore(&mut self, snapshot: SpuSnapshot) {
        let SpuSnapshot {
            state,
            status,
            stall,
            pipeline,
        } = snapshot;
        self.state = state;
        self.status = status;
        self.stall = stall;
        self.pipeline = pipeline;
        self.barriers.clear();
        self.per_step_hashes.clear();
        self.per_step_full_states.clear();
    }
}
