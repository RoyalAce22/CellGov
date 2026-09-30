//! The SPU unit's state, its constructor, and its accessors.

use crate::state;
use cellgov_event::UnitId;
use cellgov_exec::{ChannelStall, UnitStatus};

/// The SPU's whole context: its architected state and the unit's run
/// state around it. Replay, instruction comparison, the local-store
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
}

impl SpuSnapshot {
    /// The comparison view of the architected state.
    pub fn observable(&self) -> state::SpuObservableSnapshot {
        state::SpuObservableSnapshot::capture(&self.state)
    }

    /// The hash of the local store that the runtime's observable hash
    /// folds in.
    pub fn local_store_hash(&self) -> u64 {
        local_store_hash(&self.state.ls)
    }
}

/// FNV-1a over the local-store bytes.
pub(super) fn local_store_hash(ls: &[u8]) -> u64 {
    let mut hasher = cellgov_mem::Fnv1aHasher::new();
    hasher.write(ls);
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
    /// Barrier instructions retired since the last drain.
    ///
    /// The unit records a barrier only while the context asks for
    /// per-step trace data.
    pub(super) barriers: Vec<cellgov_exec::RetiredBarrier>,
}

impl SpuExecutionUnit {
    /// Construct a runnable SPU with zeroed architectural state.
    pub fn new(id: UnitId) -> Self {
        Self {
            id,
            state: state::SpuState::new(),
            status: UnitStatus::Runnable,
            stall: None,
            barriers: Vec::new(),
        }
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
    /// its id and drops the barriers it has not drained, which are trace
    /// output, not state.
    ///
    /// [CBEA p:241 s:17] a context restore returns the SPE to the saved state.
    pub fn restore(&mut self, snapshot: SpuSnapshot) {
        let SpuSnapshot {
            state,
            status,
            stall,
        } = snapshot;
        self.state = state;
        self.status = status;
        self.stall = stall;
        self.barriers.clear();
    }
}
