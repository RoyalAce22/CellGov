//! The integration tests of `cellgov_fuzz`, one module per subject, linked as
//! one test binary so each profile links the crate's dependencies once.

mod decoder_campaign_integration;
mod engine_tests;
mod finite_sweep;
mod library_shape;
mod ppu_dependency_sequences;
mod ppu_layered_reference_integration;
mod ppu_path_commit_refusal;
mod ppu_path_differential;
mod ppu_path_read_footprint;
mod ppu_path_stop_attribution;
mod ppu_reference_artifact;
mod raw_decode_campaign;
mod semantic_sweep;
mod spu_layered_reference_integration;
mod spu_observation_engine;
mod spu_reference_artifact;
mod spu_reference_campaign;
