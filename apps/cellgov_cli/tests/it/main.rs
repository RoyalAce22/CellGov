//! The integration tests of `cellgov_cli`, one module per subject, linked as
//! one test binary so each profile links the crate's dependencies once.

#[path = "../common/registry.rs"]
mod registry;

mod anchor_structure;
mod boot_firmware_floor_warning;
mod cli_reference;
mod compare_baseline_round_trip;
mod compare_observations_tripwire;
mod compare_region_refusal;
mod compare_unsupported_baseline;
mod cross_triple_warnings;
mod diverge_fixtures;
mod dma_overlay;
mod exit_code_contract;
mod fetch_versus_write;
mod firmware_install_exit_contract;
mod firmware_pup_verify;
mod fuzz_progress_purity;
mod gen_manifest_refusals;
mod readme_drift;
mod registry_structure;
mod run_game_preflight;
mod scheme_mismatch_exit;
mod snapshot_effects_tripwire;
mod spu_disasm_cli;
mod store_exit_codes;
mod titles_gen_refusals;
