//! Microtest manifest parsing.
//!
//! A manifest is a TOML file that ties a CellGov scenario to an RPCS3
//! test binary, declares memory regions to observe, and specifies the
//! expected outcome. One manifest per microtest.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::observation::ObservedOutcome;
#[cfg(feature = "rpcs3-runner")]
use crate::runner_rpcs3::Rpcs3Decoder;

/// A parsed microtest manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    /// Test identity.
    pub test: TestSection,
    /// CellGov-side configuration (absent for RPCS3-only tests).
    pub cellgov: Option<CellGovSection>,
    /// RPCS3-side configuration (absent for CellGov-only tests).
    pub rpcs3: Option<Rpcs3Section>,
    /// What to observe and compare.
    pub observe: ObserveSection,
    /// Expected outcome.
    pub expect: ExpectSection,
}

/// Top-level test identity.
#[derive(Debug, Clone, Deserialize)]
pub struct TestSection {
    /// Unique test name.
    pub name: String,
}

/// CellGov-side configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct CellGovSection {
    /// Name of a registered `ScenarioFixture` factory.
    pub scenario: String,
    /// Key-value arguments passed to the factory.
    #[serde(default)]
    pub scenario_args: BTreeMap<String, toml::Value>,
    /// Max steps for the CellGov run.
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,
}

/// RPCS3-side configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Rpcs3Section {
    /// Path to the ELF binary, relative to the manifest file.
    pub binary: String,
    /// Decoder mode.
    #[serde(default)]
    pub decoder: DecoderField,
    /// Wall-clock timeout in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

/// What to observe and compare between runners.
#[derive(Debug, Clone, Deserialize)]
pub struct ObserveSection {
    /// Memory regions to capture at end of run.
    #[serde(default)]
    pub memory_regions: Vec<MemoryRegionSpec>,
    /// Whether to capture mailbox message sequences.
    #[serde(default)]
    pub mailbox_sequences: bool,
    /// Whether to capture CellGov state hashes.
    #[serde(default)]
    pub final_hashes: bool,
    /// Event classes to include in comparison.
    #[serde(default)]
    pub event_classes: Vec<String>,
}

/// Expected outcome for the test.
#[derive(Debug, Clone, Deserialize)]
pub struct ExpectSection {
    /// Expected test outcome.
    pub outcome: OutcomeField,
}

/// A memory region to observe, as declared in the manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryRegionSpec {
    /// Region name (used in reports and baseline keys).
    pub name: String,
    /// Guest address of the region start.
    pub addr: u64,
    /// Size in bytes.
    pub size: u64,
}

/// RPCS3 decoder selection.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecoderField {
    /// PPU + SPU interpreter.
    #[default]
    Interpreter,
    /// PPU + SPU LLVM recompiler.
    Llvm,
}

#[cfg(feature = "rpcs3-runner")]
impl From<DecoderField> for Rpcs3Decoder {
    fn from(d: DecoderField) -> Self {
        match d {
            DecoderField::Interpreter => Rpcs3Decoder::Interpreter,
            DecoderField::Llvm => Rpcs3Decoder::Llvm,
        }
    }
}

/// Expected-outcome field (lowercase string in TOML).
///
/// Variant set must be in 1:1 correspondence with [`ObservedOutcome`];
/// `tests::outcome_field_and_observed_outcome_are_isomorphic` pins
/// the contract.
///
/// `ProcessExit` accepts `process_exit` and `process-exit` aliases in
/// addition to the canonical lowercase `processexit`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, strum::VariantArray)]
#[serde(rename_all = "lowercase")]
pub enum OutcomeField {
    /// Test ran to completion.
    Completed,
    /// Test stalled (deadlock or livelock).
    Stalled,
    /// Test exceeded its time or step budget.
    Timeout,
    /// Test faulted.
    Fault,
    /// Title exited via `sys_process_exit`.
    #[serde(alias = "process_exit", alias = "process-exit")]
    ProcessExit,
}

impl From<OutcomeField> for ObservedOutcome {
    fn from(o: OutcomeField) -> Self {
        match o {
            OutcomeField::Completed => ObservedOutcome::Completed,
            OutcomeField::Stalled => ObservedOutcome::Stalled,
            OutcomeField::Timeout => ObservedOutcome::Timeout,
            OutcomeField::Fault => ObservedOutcome::Fault,
            OutcomeField::ProcessExit => ObservedOutcome::ProcessExit,
        }
    }
}

fn default_max_steps() -> usize {
    10000
}

fn default_timeout_ms() -> u64 {
    5000
}

/// Why manifest loading failed.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// File system error.
    #[error("manifest I/O: {0}")]
    Io(#[from] std::io::Error),
    /// TOML parse error.
    #[error("manifest parse: {0}")]
    Parse(#[from] toml::de::Error),
}

/// Load and parse a manifest from a TOML file.
pub fn load(path: &Path) -> Result<Manifest, ManifestError> {
    let text = std::fs::read_to_string(path)?;
    parse(&text)
}

/// Parse a manifest from a TOML string.
pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
    let manifest: Manifest = toml::from_str(text)?;
    Ok(manifest)
}

#[cfg(test)]
#[path = "tests/manifest_tests.rs"]
mod tests;
