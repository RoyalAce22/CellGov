//! Supervisor state: the registers and the privileged forms that the
//! console's own LV2 kernel runs with MSR\[PR\] = 0.

mod exec;
mod state;

pub(crate) use exec::{execute, moves_decrementer};
pub use state::{SlbEntry, SupervisorState};

#[cfg(test)]
#[path = "tests/exec_tests.rs"]
mod exec_tests;
