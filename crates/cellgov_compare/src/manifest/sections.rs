//! Section structs that make up the manifest schema.

use serde::Deserialize;
use std::collections::BTreeMap;

use super::fields::{
    default_max_steps, default_timeout_ms, DecoderField, MemoryRegionSpec, OutcomeField,
};

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
