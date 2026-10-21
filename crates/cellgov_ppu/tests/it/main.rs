//! The integration tests of `cellgov_ppu`, one module per subject, linked as
//! one test binary so each profile links the crate's dependencies once.

mod console_ppu_issue_chain;
mod console_ppu_issue_layout;
mod console_ppu_issue_loop_shape;
mod console_ppu_issue_multiple;
mod console_ppu_issue_multiply;
mod console_ppu_issue_priority;
mod console_ppu_issue_shared;
mod console_ppu_issue_sibling_rate;
mod console_ppu_issue_smt;
mod console_ppu_issue_timing;
mod console_ppu_issue_units;
mod reloc_addr16_lo_ds;
mod reloc_addr64;
mod reloc_rel24;
mod snapshot_shadow_independence;
mod sprx_error_coverage;
