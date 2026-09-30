//! The stopped state a unit reports when its own instruction stops it,
//! and the refusals a restart can meet.

/// A unit's stopped state, in the two words the SPE problem-state
/// registers report it in.
///
/// [CBEA p:93 s:8.5.2] SPU_Status reports why the SPU stopped and the stop code.
/// [CBEA p:95 s:8.5.3] SPU_NPC holds the address the SPU resumes at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StopRegisters {
    /// The `SPU_Status` word, with the run bit clear.
    pub status: u32,
    /// The `SPU_NPC` word: the local-store address the unit resumes at.
    pub npc: u32,
}

/// Why a unit refused a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RestartError {
    /// The unit did not stop itself, so there is no stopped state to
    /// resume from.
    #[error("unit is not stopped")]
    NotStopped,
    /// The runtime holds no unit with the id.
    #[error("no unit with that id")]
    UnknownUnit,
    /// A `Finished` status override holds the unit, as a process exit
    /// sets for every unit of the process.
    #[error("unit is retired by a status override")]
    Retired,
}
