//! The SPU unit's state, its constructor, and its accessors.

use crate::state;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

/// SPU execution unit snapshot for replay.
#[derive(Debug, Clone)]
pub struct SpuSnapshot {
    /// Register file.
    pub regs: [[u8; 16]; 128],
    /// Program counter.
    pub pc: u32,
    /// Local store contents.
    pub ls: Vec<u8>,
    /// Canonical line address of the atomic reservation; `None` when
    /// no reservation is held.
    pub reservation_line: Option<u64>,
}

/// A Synergistic Processing Unit execution unit.
#[derive(Clone)]
pub struct SpuExecutionUnit {
    pub(super) id: UnitId,
    pub(super) state: state::SpuState,
    pub(super) status: UnitStatus,
}

impl SpuExecutionUnit {
    /// Construct a runnable SPU with zeroed architectural state.
    pub fn new(id: UnitId) -> Self {
        Self {
            id,
            state: state::SpuState::new(),
            status: UnitStatus::Runnable,
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
}
