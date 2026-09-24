//! The PPU execution unit: its state, the batch loop, fetch-run
//! tracking, fault rollback, and the `ExecutionUnit` impl.

mod batch;
mod exec_unit;
mod fault;
mod fetch;
mod ppu_unit;

pub use ppu_unit::{PpuExecutionUnit, PpuSnapshot};

#[cfg(test)]
#[path = "tests/ppu_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/break_pc_tests.rs"]
mod break_pc_tests;

#[cfg(test)]
#[path = "tests/batch_fault_tests.rs"]
mod batch_fault_tests;

#[cfg(test)]
#[path = "tests/tap_tests.rs"]
mod tap_tests;

#[cfg(test)]
#[path = "tests/clock_read_tests.rs"]
mod clock_read_tests;
