//! Microtest manifest parsing.
//!
//! Each manifest is a TOML file that ties together a CellGov scenario
//! and an RPCS3 test binary, declares which memory regions to observe,
//! and specifies the expected outcome. The harness reads one manifest
//! per microtest.

use crate::observation::ObservedOutcome;
use crate::runner_rpcs3::Rpcs3Decoder;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

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
    /// Name of a registered ScenarioFixture factory.
    pub scenario: String,
    /// Key-value arguments passed to the factory.
    #[serde(default)]
    pub scenario_args: BTreeMap<String, toml::Value>,
    /// Max steps for the CellGov run. Defaults to 10000.
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,
}

fn default_max_steps() -> usize {
    10000
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

fn default_timeout_ms() -> u64 {
    5000
}

/// Decoder field that deserializes from a string.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecoderField {
    /// PPU Interpreter + SPU Interpreter.
    #[default]
    Interpreter,
    /// PPU LLVM Recompiler + SPU LLVM Recompiler.
    Llvm,
}

impl From<DecoderField> for Rpcs3Decoder {
    fn from(d: DecoderField) -> Self {
        match d {
            DecoderField::Interpreter => Rpcs3Decoder::Interpreter,
            DecoderField::Llvm => Rpcs3Decoder::Llvm,
        }
    }
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

/// Expected outcome for the test.
#[derive(Debug, Clone, Deserialize)]
pub struct ExpectSection {
    /// Expected test outcome.
    pub outcome: OutcomeField,
}

/// Outcome field that deserializes from a lowercase string.
#[derive(Debug, Clone, Deserialize)]
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
}

impl From<OutcomeField> for ObservedOutcome {
    fn from(o: OutcomeField) -> Self {
        match o {
            OutcomeField::Completed => ObservedOutcome::Completed,
            OutcomeField::Stalled => ObservedOutcome::Stalled,
            OutcomeField::Timeout => ObservedOutcome::Timeout,
            OutcomeField::Fault => ObservedOutcome::Fault,
        }
    }
}

/// Why manifest loading failed.
#[derive(Debug)]
pub enum ManifestError {
    /// File system error.
    Io(std::io::Error),
    /// TOML parse error.
    Parse(toml::de::Error),
}

impl From<std::io::Error> for ManifestError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<toml::de::Error> for ManifestError {
    fn from(e: toml::de::Error) -> Self {
        Self::Parse(e)
    }
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
mod tests {
    use super::*;

    const FULL_MANIFEST: &str = r#"
[test]
name = "mailbox_roundtrip_001"

[cellgov]
scenario = "mailbox_roundtrip"
scenario_args = { command = 66 }
max_steps = 10000

[rpcs3]
binary = "tests/micro/mailbox_roundtrip/spu.elf"
decoder = "interpreter"
timeout_ms = 5000

[observe]
memory_regions = [
  { name = "result", addr = 65536, size = 64 },
]
mailbox_sequences = true
final_hashes = true
event_classes = ["mailbox", "dma", "wakeup"]

[expect]
outcome = "completed"
"#;

    #[test]
    fn parse_full_manifest() {
        let m = parse(FULL_MANIFEST).expect("parse");
        assert_eq!(m.test.name, "mailbox_roundtrip_001");

        let cg = m.cellgov.unwrap();
        assert_eq!(cg.scenario, "mailbox_roundtrip");
        assert_eq!(cg.max_steps, 10000);
        assert_eq!(
            cg.scenario_args.get("command").and_then(|v| v.as_integer()),
            Some(66)
        );

        let rpcs3 = m.rpcs3.unwrap();
        assert_eq!(rpcs3.binary, "tests/micro/mailbox_roundtrip/spu.elf");
        assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
        assert_eq!(rpcs3.timeout_ms, 5000);

        assert_eq!(m.observe.memory_regions.len(), 1);
        assert_eq!(m.observe.memory_regions[0].name, "result");
        assert_eq!(m.observe.memory_regions[0].addr, 65536);
        assert_eq!(m.observe.memory_regions[0].size, 64);
        assert!(m.observe.mailbox_sequences);
        assert!(m.observe.final_hashes);
        assert_eq!(m.observe.event_classes, vec!["mailbox", "dma", "wakeup"]);

        assert!(matches!(m.expect.outcome, OutcomeField::Completed));
    }

    #[test]
    fn parse_cellgov_only_manifest() {
        let text = r#"
[test]
name = "cellgov_only"

[cellgov]
scenario = "fairness"

[observe]
memory_regions = []

[expect]
outcome = "completed"
"#;
        let m = parse(text).expect("parse");
        assert!(m.cellgov.is_some());
        assert!(m.rpcs3.is_none());
        // Defaults
        assert_eq!(m.cellgov.unwrap().max_steps, 10000);
    }

    #[test]
    fn parse_rpcs3_only_manifest() {
        let text = r#"
[test]
name = "rpcs3_only"

[rpcs3]
binary = "test.elf"
decoder = "llvm"
timeout_ms = 3000

[observe]
memory_regions = []

[expect]
outcome = "completed"
"#;
        let m = parse(text).expect("parse");
        assert!(m.cellgov.is_none());
        assert!(m.rpcs3.is_some());
        let rpcs3 = m.rpcs3.unwrap();
        assert!(matches!(rpcs3.decoder, DecoderField::Llvm));
        assert_eq!(rpcs3.timeout_ms, 3000);
    }

    #[test]
    fn defaults_apply_when_fields_omitted() {
        let text = r#"
[test]
name = "defaults"

[cellgov]
scenario = "isa"

[rpcs3]
binary = "test.elf"

[observe]

[expect]
outcome = "completed"
"#;
        let m = parse(text).expect("parse");
        assert_eq!(m.cellgov.unwrap().max_steps, 10000);
        let rpcs3 = m.rpcs3.unwrap();
        assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
        assert_eq!(rpcs3.timeout_ms, 5000);
        assert!(m.observe.memory_regions.is_empty());
        assert!(!m.observe.mailbox_sequences);
        assert!(!m.observe.final_hashes);
    }

    #[test]
    fn all_outcome_variants_parse() {
        for (text, expected) in [
            ("completed", "Completed"),
            ("stalled", "Stalled"),
            ("timeout", "Timeout"),
            ("fault", "Fault"),
        ] {
            let toml_text = format!(
                r#"
[test]
name = "outcome_test"
[observe]
[expect]
outcome = "{text}"
"#
            );
            let m = parse(&toml_text).expect("parse");
            let mapped: ObservedOutcome = m.expect.outcome.into();
            assert_eq!(format!("{mapped:?}"), expected);
        }
    }

    #[test]
    fn invalid_toml_returns_error() {
        let result = parse("not valid toml {{{}}}");
        assert!(result.is_err());
    }

    #[test]
    fn missing_required_field_returns_error() {
        let result = parse("[test]\n[observe]\n[expect]\noutcome = \"completed\"");
        // Missing test.name
        assert!(result.is_err());
    }

    #[test]
    fn load_spu_mailbox_write_manifest() {
        let path = std::path::Path::new("../../tests/micro/spu_mailbox_write/manifest.toml");
        if path.exists() {
            let m = load(path).expect("load manifest");
            assert_eq!(m.test.name, "spu_mailbox_write");
            assert!(m.cellgov.is_some());
            assert!(m.rpcs3.is_none()); // RPCS3 section is commented out
        }
    }

    #[test]
    fn load_spu_fixed_value_manifest() {
        let path = std::path::Path::new("../../tests/micro/spu_fixed_value/manifest.toml");
        if path.exists() {
            let m = load(path).expect("load manifest");
            assert_eq!(m.test.name, "spu_fixed_value");
            assert!(m.cellgov.is_none()); // RPCS3-only test
            assert!(m.rpcs3.is_some());
            let rpcs3 = m.rpcs3.unwrap();
            assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
            assert_eq!(m.observe.memory_regions.len(), 1);
            assert_eq!(m.observe.memory_regions[0].name, "result");
            assert_eq!(m.observe.memory_regions[0].size, 8);
        }
    }

    #[test]
    fn load_atomic_reservation_manifest() {
        let path = std::path::Path::new("../../tests/micro/atomic_reservation/manifest.toml");
        if path.exists() {
            let m = load(path).expect("load manifest");
            assert_eq!(m.test.name, "atomic_reservation");
            assert!(m.cellgov.is_none());
            assert!(m.rpcs3.is_some());
            let rpcs3 = m.rpcs3.unwrap();
            assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
            assert_eq!(m.observe.memory_regions.len(), 2);
            assert_eq!(m.observe.memory_regions[0].name, "header");
            assert_eq!(m.observe.memory_regions[0].size, 8);
            assert_eq!(m.observe.memory_regions[1].name, "data");
            assert_eq!(m.observe.memory_regions[1].size, 128);
        }
    }

    #[test]
    fn load_ls_to_shared_manifest() {
        let path = std::path::Path::new("../../tests/micro/ls_to_shared/manifest.toml");
        if path.exists() {
            let m = load(path).expect("load manifest");
            assert_eq!(m.test.name, "ls_to_shared");
            assert!(m.cellgov.is_none());
            assert!(m.rpcs3.is_some());
            let rpcs3 = m.rpcs3.unwrap();
            assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
            assert_eq!(m.observe.memory_regions.len(), 2);
            assert_eq!(m.observe.memory_regions[0].name, "header");
            assert_eq!(m.observe.memory_regions[0].size, 8);
            assert_eq!(m.observe.memory_regions[1].name, "data");
            assert_eq!(m.observe.memory_regions[1].size, 128);
        }
    }

    #[test]
    fn load_barrier_wakeup_manifest() {
        let path = std::path::Path::new("../../tests/micro/barrier_wakeup/manifest.toml");
        if path.exists() {
            let m = load(path).expect("load manifest");
            assert_eq!(m.test.name, "barrier_wakeup");
            assert!(m.cellgov.is_none());
            assert!(m.rpcs3.is_some());
            let rpcs3 = m.rpcs3.unwrap();
            assert!(matches!(rpcs3.decoder, DecoderField::Interpreter));
            assert_eq!(m.observe.memory_regions.len(), 2);
            assert_eq!(m.observe.memory_regions[0].name, "spu0_result");
            assert_eq!(m.observe.memory_regions[0].size, 8);
            assert_eq!(m.observe.memory_regions[1].name, "spu1_result");
            assert_eq!(m.observe.memory_regions[1].size, 8);
        }
    }
}
