//! The PPU and SPU descriptor registries, built once per process.
//!
//! Each registry is a pure function of the build, and one build takes
//! tens of milliseconds in a debug build. Every campaign reads a
//! registry, and so does the one-case campaign of each replay, reduction
//! step and case-word lookup.

use std::sync::OnceLock;

use cellgov_ppu::instruction::fuzz::PpuGenerationDescriptor;
use cellgov_spu::fuzz::SpuGenerationDescriptor;

/// The PPU generation descriptors, built on first use.
///
/// A panic while building leaves the registry unbuilt, so the next call
/// builds again and meets the same panic at its own target boundary.
pub(crate) fn ppu_descriptors() -> Vec<PpuGenerationDescriptor> {
    static REGISTRY: OnceLock<Vec<PpuGenerationDescriptor>> = OnceLock::new();
    REGISTRY
        .get_or_init(cellgov_ppu::instruction::fuzz::generation_descriptors)
        .clone()
}

/// The SPU generation descriptors, built on first use.
///
/// A panic while building leaves the registry unbuilt, so the next call
/// builds again and meets the same panic at its own target boundary.
pub(crate) fn spu_descriptors() -> Vec<SpuGenerationDescriptor> {
    static REGISTRY: OnceLock<Vec<SpuGenerationDescriptor>> = OnceLock::new();
    REGISTRY
        .get_or_init(cellgov_spu::fuzz::generation_descriptors)
        .clone()
}

#[cfg(test)]
#[path = "tests/registry_tests.rs"]
mod tests;
