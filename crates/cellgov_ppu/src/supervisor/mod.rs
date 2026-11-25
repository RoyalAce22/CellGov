//! Supervisor state: the registers and the privileged forms that the
//! console's own LV2 kernel runs with MSR\[PR\] = 0.

mod exec;
mod facility;
mod state;

pub(crate) use exec::{execute, moves_decrementer};
pub(crate) use facility::unavailable;
pub use state::{SlbEntry, SupervisorState};

#[cfg(test)]
#[path = "tests/exec_tests.rs"]
mod exec_tests;

#[cfg(test)]
#[path = "tests/bp_vr_tests.rs"]
mod bp_vr_tests;

#[cfg(test)]
#[path = "tests/facility_tests.rs"]
mod facility_tests;

#[cfg(test)]
#[path = "tests/mtmsr_trace_tests.rs"]
mod mtmsr_trace_tests;
