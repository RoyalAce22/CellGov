//! The PPU list built from the documents alone.
//!
//! The module owns:
//! - input vectors that reach every result the documents leave open,
//!   with controls the documents fix beside them;
//! - for each component, the page that fixes it or leaves it open;
//! - the probe's case table, a replay of the list through the
//!   interpreter, and the comparison of a console capture with both.
//!
//! No expected value comes from a console, an emulator or a host
//! floating-point unit.

mod capture;
mod encode;
mod generate;
mod replay;
mod set;
mod vector;

pub use crate::spu_documented::digest;
pub use capture::{
    compare_capture, read_frame, slots, CaptureReport, ConsoleCapture, ConsoleRun, Disagreement,
    FrameError,
};
pub use generate::documented_vectors;
pub use replay::{
    replay_documented, run_vector, DocumentedDifference, DocumentedReplay, StoppedVector,
    MEM_BASE_GPR, MEM_BYTES, REPLAY_MEM_BASE,
};
pub use set::Set;
pub use vector::{
    case_table, render, to_json_line, Component, DocumentedVector, Location, Verdict,
};

#[cfg(test)]
#[path = "../tests/ppu_documented_tests.rs"]
mod tests;
