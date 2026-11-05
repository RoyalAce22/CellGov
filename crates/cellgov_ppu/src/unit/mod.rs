//! The PPU execution unit: its state, the batch loop, fetch-run
//! tracking, fault rollback, and the `ExecutionUnit` impl.

mod batch;
mod code_events;
mod exec_unit;
mod fault;
mod fetch;
mod load_probe;
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
#[path = "tests/sibling_lead_tests.rs"]
mod sibling_lead_tests;

#[cfg(test)]
#[path = "tests/barrier_tests.rs"]
mod barrier_tests;

#[cfg(test)]
#[path = "tests/window_access_tests.rs"]
mod window_access_tests;

#[cfg(test)]
#[path = "tests/window_fused_pair_tests.rs"]
mod window_fused_pair_tests;

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
#[path = "tests/fused_lis_entry_tests.rs"]
mod fused_lis_entry_tests;

#[cfg(test)]
#[path = "tests/fused_ld_pair_tests.rs"]
mod fused_ld_pair_tests;

#[cfg(test)]
#[path = "tests/fused_ld_provisional_tests.rs"]
mod fused_ld_provisional_tests;

#[cfg(test)]
#[path = "tests/fused_lbzu_cmpwi_tests.rs"]
mod fused_lbzu_cmpwi_tests;

#[cfg(test)]
#[path = "tests/fused_extsw_blr_tests.rs"]
mod fused_extsw_blr_tests;

#[cfg(test)]
#[path = "tests/fused_ldu_pair_tests.rs"]
mod fused_ldu_pair_tests;

#[cfg(test)]
#[path = "tests/fused_cmpd_nor_tests.rs"]
mod fused_cmpd_nor_tests;

#[cfg(test)]
#[path = "tests/fused_addi_bc_tests.rs"]
mod fused_addi_bc_tests;

#[cfg(test)]
#[path = "tests/fused_cmp_bc_self_tests.rs"]
mod fused_cmp_bc_self_tests;

#[cfg(test)]
#[path = "tests/fused_ld_rlwinm_tests.rs"]
mod fused_ld_rlwinm_tests;

#[cfg(test)]
#[path = "tests/fused_lwz_pair_tests.rs"]
mod fused_lwz_pair_tests;

#[cfg(test)]
#[path = "tests/fused_or_rlwinm_tests.rs"]
mod fused_or_rlwinm_tests;

#[cfg(test)]
#[path = "tests/fused_std_lwz_tests.rs"]
mod fused_std_lwz_tests;

#[cfg(test)]
#[path = "tests/fused_ld_any_tests.rs"]
mod fused_ld_any_tests;

#[cfg(test)]
#[path = "tests/fused_or_b_tests.rs"]
mod fused_or_b_tests;

#[cfg(test)]
#[path = "tests/fused_lbz_pair_tests.rs"]
mod fused_lbz_pair_tests;

#[cfg(test)]
#[path = "tests/fused_mtlr_addi_tests.rs"]
mod fused_mtlr_addi_tests;

#[cfg(test)]
#[path = "tests/fused_rldicl_pair_tests.rs"]
mod fused_rldicl_pair_tests;

#[cfg(test)]
#[path = "tests/fused_ori_and_tests.rs"]
mod fused_ori_and_tests;

#[cfg(test)]
#[path = "tests/fused_multi_form_tests.rs"]
mod fused_multi_form_tests;

#[cfg(test)]
#[path = "tests/lbz_lbz_console_tests.rs"]
mod lbz_lbz_console_tests;

#[cfg(test)]
#[path = "tests/mtlr_addi_console_tests.rs"]
mod mtlr_addi_console_tests;

#[cfg(test)]
#[path = "tests/rldicl_rldicl_console_tests.rs"]
mod rldicl_rldicl_console_tests;

#[cfg(test)]
#[path = "tests/ori_and_console_tests.rs"]
mod ori_and_console_tests;

#[cfg(test)]
#[path = "tests/std_lwz_console_tests.rs"]
mod std_lwz_console_tests;

#[cfg(test)]
#[path = "tests/ld_ld_console_tests.rs"]
mod ld_ld_console_tests;

#[cfg(test)]
#[path = "tests/ld_any_console_tests.rs"]
mod ld_any_console_tests;

#[cfg(test)]
#[path = "tests/lwz_lwz_console_tests.rs"]
mod lwz_lwz_console_tests;

#[cfg(test)]
#[path = "tests/ld_rlwinm_console_tests.rs"]
mod ld_rlwinm_console_tests;

#[cfg(test)]
#[path = "tests/ldu_ldu_console_tests.rs"]
mod ldu_ldu_console_tests;

#[cfg(test)]
#[path = "tests/lbzu_cmpwi_console_tests.rs"]
mod lbzu_cmpwi_console_tests;

#[cfg(test)]
#[path = "tests/lis_ori_console_tests.rs"]
mod lis_ori_console_tests;

#[cfg(test)]
#[path = "tests/or_add_console_tests.rs"]
mod or_add_console_tests;

#[cfg(test)]
#[path = "tests/or_rlwinm_console_tests.rs"]
mod or_rlwinm_console_tests;

#[cfg(test)]
#[path = "tests/cmpd_nor_console_tests.rs"]
mod cmpd_nor_console_tests;

#[cfg(test)]
#[path = "tests/addi_bc_console_tests.rs"]
mod addi_bc_console_tests;

#[cfg(test)]
#[path = "tests/multi_form_reg_console_tests.rs"]
mod multi_form_reg_console_tests;

#[cfg(test)]
#[path = "tests/multi_form_load_console_tests.rs"]
mod multi_form_load_console_tests;

#[cfg(test)]
#[path = "tests/store_fault_tests.rs"]
mod store_fault_tests;

#[cfg(test)]
#[path = "tests/operand_profile_tests.rs"]
mod operand_profile_tests;

#[cfg(test)]
#[path = "tests/load_probe_tests.rs"]
mod load_probe_tests;

#[cfg(test)]
#[path = "tests/interrupt_record_tests.rs"]
mod interrupt_record_tests;

#[cfg(test)]
#[path = "tests/fetch_attribute_tests.rs"]
mod fetch_attribute_tests;

#[cfg(test)]
#[path = "tests/fetch_region_tests.rs"]
mod fetch_region_tests;

#[cfg(test)]
#[path = "tests/synced_code_store_tests.rs"]
mod synced_code_store_tests;

#[cfg(test)]
#[path = "tests/committed_code_write_tests.rs"]
mod committed_code_write_tests;

#[cfg(test)]
#[path = "tests/code_write_record_tests.rs"]
mod code_write_record_tests;

#[cfg(test)]
#[path = "tests/snapshot_restore_tests.rs"]
mod snapshot_restore_tests;

#[cfg(test)]
#[path = "tests/code_event_tests.rs"]
mod code_event_tests;

#[cfg(test)]
#[path = "tests/code_event_atomic_tests.rs"]
mod code_event_atomic_tests;
