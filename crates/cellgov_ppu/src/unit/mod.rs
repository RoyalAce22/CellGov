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
#[path = "tests/reference_support.rs"]
mod reference_support;

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

#[cfg(test)]
#[path = "tests/time_base_tests.rs"]
mod time_base_tests;

#[cfg(test)]
#[path = "tests/barrier_tests.rs"]
mod barrier_tests;

#[cfg(test)]
#[path = "tests/window_access_tests.rs"]
mod window_access_tests;

#[cfg(test)]
#[path = "tests/fused_entry_tests.rs"]
mod fused_entry_tests;

#[cfg(test)]
#[path = "tests/top_of_address_space_tests.rs"]
mod top_of_address_space_tests;

#[cfg(test)]
#[path = "tests/fused_resume_invalidation_tests.rs"]
mod fused_resume_invalidation_tests;

#[cfg(test)]
#[path = "tests/fused_trace_hash_tests.rs"]
mod fused_trace_hash_tests;

#[cfg(test)]
#[path = "tests/sc_reserved_lev_tests.rs"]
mod sc_reserved_lev_tests;

#[cfg(test)]
#[path = "tests/sc_retirement_tests.rs"]
mod sc_retirement_tests;

#[cfg(test)]
#[path = "tests/smc_branch_patch_tests.rs"]
mod smc_branch_patch_tests;

#[cfg(test)]
#[path = "tests/fused_entry_console_tests.rs"]
mod fused_entry_console_tests;

#[cfg(test)]
#[path = "tests/fault_prefix_tests.rs"]
mod fault_prefix_tests;

#[cfg(test)]
#[path = "tests/witness_fault_prefix_tests.rs"]
mod witness_fault_prefix_tests;

#[cfg(test)]
#[path = "tests/fused_refusal_tests.rs"]
mod fused_refusal_tests;

#[cfg(test)]
#[path = "tests/store_fault_tests.rs"]
mod store_fault_tests;

#[cfg(test)]
#[path = "tests/interrupt_record_tests.rs"]
mod interrupt_record_tests;

#[cfg(test)]
#[path = "tests/fetch_attribute_tests.rs"]
mod fetch_attribute_tests;

#[cfg(test)]
#[path = "tests/synced_code_store_tests.rs"]
mod synced_code_store_tests;

#[cfg(test)]
#[path = "tests/committed_code_write_tests.rs"]
mod committed_code_write_tests;

#[cfg(test)]
#[path = "tests/snapshot_restore_tests.rs"]
mod snapshot_restore_tests;
