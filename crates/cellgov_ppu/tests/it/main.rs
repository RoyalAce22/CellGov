//! The integration tests of `cellgov_ppu`, one module per subject, linked as
//! one test binary so each profile links the crate's dependencies once.

mod reloc_addr16_lo_ds;
mod reloc_addr64;
mod reloc_rel24;
mod snapshot_shadow_independence;
mod sprx_error_coverage;
