//! The SPU floating-point list built from the documents alone.
//!
//! The module owns:
//! - input vectors that reach every result the documents fix or leave open;
//! - for each component, the page that fixes it or the question that
//!   leaves it open;
//! - a replay of the list through the interpreter.
//!
//! No expected value comes from a console, an emulator or a host
//! floating-point unit. This module does not run the list on a console.

mod dyadic;
mod generate;
mod model;
mod question;
mod replay;
mod vector;

pub use generate::documented_vectors;
pub use question::{Question, VectorClass};
pub use replay::{
    replay_documented, run_vector, DocumentedDifference, DocumentedReplay, StoppedVector,
};
pub use vector::{digest, render, to_json_line, Component, DocumentedVector, Location, Verdict};

#[cfg(test)]
#[path = "../tests/spu_documented_tests.rs"]
mod tests;
