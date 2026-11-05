//! Replay of an RPCS3 per-instruction PPU trace through CellGov, with a
//! report per decoded kind and a class for every failing record.
//!
//! RPCS3 is a peer, so a record CellGov disagrees with is not a CellGov
//! defect by that fact alone. Each failure takes the first class that
//! fits:
//! - the documents leave every differing bit undefined;
//! - a console vector with the record's inputs decides which side is
//!   wrong;
//! - the record depends on state one instruction's replay cannot
//!   restore;
//! - none of these, left for a person to triage.
//!
//! [McKeeman1998 p:101 s:Seeking an Oracle] A check that a result is unchanged serves no purpose unless the result is known to be correct.

mod console;
mod replay;

pub use console::{console_vectors, ConsoleSide, ConsoleVector, PpuTraceReferenceError};
pub use replay::{
    differing_fields, kind_name, replay_trace, PpuTraceClass, PpuTraceFailure, PpuTraceKindRow,
    PpuTraceOptions, PpuTraceReport,
};

#[cfg(test)]
#[path = "../tests/ppu_trace_tests.rs"]
mod tests;
