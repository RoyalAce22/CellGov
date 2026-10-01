//! Microtest manifest parsing.
//!
//! A manifest is a TOML file that ties a CellGov scenario to an RPCS3
//! test binary, declares memory regions to observe, and specifies the
//! expected outcome. One manifest per microtest.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::observation::ObservedOutcome;

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
    /// What the console runner needs. Every field has a default, so a
    /// manifest without the table is complete.
    #[serde(default)]
    pub ps3: Ps3Section,
}

impl Manifest {
    /// The file name the PPU program writes its CGOV frame to on the
    /// console: `[ps3] result_file`, or `cgov_<name>.bin` from the test
    /// name.
    pub fn result_file_name(&self) -> String {
        result_file_name(&self.test, &self.ps3)
    }
}

/// The tables the console runner reads from a microtest manifest.
///
/// The parse skips a `[cellgov]` table whatever its shape, so a
/// manifest that also drives `boot run --title-manifest` parses here
/// while [`Manifest`] refuses it for its missing `scenario`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConsoleManifest {
    /// Test identity.
    pub test: TestSection,
    /// What to observe and compare.
    pub observe: ObserveSection,
    /// Expected outcome.
    pub expect: ExpectSection,
    /// What the console runner needs.
    #[serde(default)]
    pub ps3: Ps3Section,
}

impl ConsoleManifest {
    /// The file name the PPU program writes its CGOV frame to on the
    /// console: `[ps3] result_file`, or `cgov_<name>.bin` from the test
    /// name.
    pub fn result_file_name(&self) -> String {
        result_file_name(&self.test, &self.ps3)
    }
}

fn result_file_name(test: &TestSection, ps3: &Ps3Section) -> String {
    ps3.result_file
        .clone()
        .unwrap_or_else(|| format!("cgov_{}.bin", test.name))
}

/// Whether `value` names one file or directory entry: not empty, not
/// `.` or `..`, and free of path separators. The runner joins each
/// name under a directory of its own, so anything else would reach
/// outside it.
fn is_bare_name(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\'])
}

/// The `[ps3]` refusals: a zero budget, a name that is not a bare
/// file name, a non-portable test with no reason, and a volatile
/// range that names no declared region, covers no bytes, or runs past
/// its region.
fn check_ps3(observe: &ObserveSection, ps3: &Ps3Section) -> Result<(), ManifestError> {
    if ps3.timeout_ms == 0 {
        return Err(ManifestError::ZeroTimeout);
    }
    let names = std::iter::once(("appid", ps3.appid.as_str()))
        .chain(ps3.result_file.iter().map(|f| ("result_file", f.as_str())))
        .chain(ps3.files.iter().map(|f| ("files", f.as_str())));
    for (field, value) in names {
        if !is_bare_name(value) {
            return Err(ManifestError::NotABareName {
                field,
                value: value.to_string(),
            });
        }
    }
    if !ps3.portable && ps3.not_portable_reason.is_none() {
        return Err(ManifestError::NotPortableWithoutReason);
    }
    for (index, range) in ps3.volatile.iter().enumerate() {
        let Some(region) = observe
            .memory_regions
            .iter()
            .find(|r| r.name == range.region)
        else {
            return Err(ManifestError::VolatileRegionUnknown {
                index,
                region: range.region.clone(),
            });
        };
        if range.size == 0 {
            return Err(ManifestError::VolatileRangeEmpty {
                index,
                region: range.region.clone(),
            });
        }
        match range.offset.checked_add(range.size) {
            Some(end) if end <= region.size => {}
            _ => {
                return Err(ManifestError::VolatileRangeOutsideRegion {
                    index,
                    region: range.region.clone(),
                    offset: range.offset,
                    size: range.size,
                    region_size: region.size,
                })
            }
        }
    }
    Ok(())
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

/// What the console runner needs to run one microtest on a PS3.
///
/// [`parse`] refuses a table that names a field this struct does not
/// have, gives no time, names a file by anything but a bare name,
/// turns portability off without a reason, or declares a volatile
/// range outside its region.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ps3Section {
    /// TITLE_ID the packaged EBOOT installs under.
    pub appid: String,
    /// Wall-clock budget for one run on the console, in milliseconds.
    pub timeout_ms: u64,
    /// File name the PPU program writes its CGOV frame to under the
    /// console's result directory. `None` derives it from the test
    /// name; see [`Manifest::result_file_name`].
    pub result_file: Option<String>,
    /// Whether the test should run on a retail console at all.
    pub portable: bool,
    /// Why it is not, when `portable` is false.
    pub not_portable_reason: Option<String>,
    /// Files deployed beside the EBOOT, by name under `build/ps3/`:
    /// the programs the PPU opens under `/app_home/`.
    pub files: Vec<String>,
    /// Bytes a hardware run legitimately varies. A comparison blanks
    /// them on both sides first.
    pub volatile: Vec<VolatileRange>,
}

impl Default for Ps3Section {
    fn default() -> Self {
        Self {
            appid: default_ps3_appid(),
            timeout_ms: default_ps3_timeout_ms(),
            result_file: None,
            portable: true,
            not_portable_reason: None,
            files: Vec::new(),
            volatile: Vec::new(),
        }
    }
}

/// A byte range inside an observed region that a hardware run may vary.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolatileRange {
    /// Name of the `[observe] memory_regions` entry the range lies in.
    pub region: String,
    /// Offset of the first byte from the region start.
    pub offset: u64,
    /// Number of bytes.
    pub size: u64,
    /// What varies there, for the reader of a comparison report.
    pub reason: String,
}

/// A memory region to observe, as declared in the manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryRegionSpec {
    /// Region name (used in reports and baseline keys).
    pub name: String,
    /// Address space the region lives in; 0 is the boot process's
    /// space and the default. A spawned child's space is numbered in
    /// spawn order from 1. RPCS3 captures hold space 0 only.
    #[serde(default)]
    pub space: u32,
    /// Guest address of the region start.
    pub addr: u64,
    /// Size in bytes.
    pub size: u64,
    /// Where the region's bytes sit in the CGOV frame payload, for the
    /// console runner, which reads the frame instead of guest memory.
    /// `None` takes [`Self::addr`], the shape of a test that emits its
    /// result struct from guest address 0.
    #[serde(default)]
    pub payload_offset: Option<u64>,
}

impl MemoryRegionSpec {
    /// The region's offset in the CGOV frame payload: `payload_offset`,
    /// or `addr` when the manifest gives none.
    pub fn payload_offset(&self) -> u64 {
        self.payload_offset.unwrap_or(self.addr)
    }
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

fn default_ps3_appid() -> String {
    "CGOV00001".to_string()
}

fn default_ps3_timeout_ms() -> u64 {
    30_000
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
    /// `[ps3] timeout_ms = 0`, which would time every run out at once.
    #[error("manifest [ps3]: timeout_ms must be greater than zero")]
    ZeroTimeout,
    /// A `[ps3]` value that must be one file or directory name is empty,
    /// `.`, `..`, or carries a path separator.
    #[error("manifest [ps3]: {field} value {value:?} is not a bare file name")]
    NotABareName {
        /// The field: `appid`, `result_file` or `files`.
        field: &'static str,
        /// The value as written.
        value: String,
    },
    /// `[ps3] portable = false` with no `not_portable_reason`.
    #[error("manifest [ps3]: portable = false needs a not_portable_reason")]
    NotPortableWithoutReason,
    /// A volatile range names a region `[observe]` does not declare.
    #[error(
        "manifest [ps3]: volatile range {index} names region {region:?}, which [observe] does not declare"
    )]
    VolatileRegionUnknown {
        /// Position of the range in `[ps3] volatile`.
        index: usize,
        /// The region name the range gave.
        region: String,
    },
    /// A volatile range covers no bytes.
    #[error("manifest [ps3]: volatile range {index} in region {region:?} is empty")]
    VolatileRangeEmpty {
        /// Position of the range in `[ps3] volatile`.
        index: usize,
        /// The region the range names.
        region: String,
    },
    /// A volatile range runs past the end of its region.
    #[error(
        "manifest [ps3]: volatile range {index} at offset {offset} of {size} bytes runs past region {region:?} of {region_size} bytes"
    )]
    VolatileRangeOutsideRegion {
        /// Position of the range in `[ps3] volatile`.
        index: usize,
        /// The region the range names.
        region: String,
        /// The range's first byte, from the region start.
        offset: u64,
        /// The range's byte count.
        size: u64,
        /// The region's byte count.
        region_size: u64,
    },
}

/// Load and parse a manifest from a TOML file.
pub fn load(path: &Path) -> Result<Manifest, ManifestError> {
    let text = std::fs::read_to_string(path)?;
    parse(&text)
}

/// Parse a manifest from a TOML string.
pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
    let manifest: Manifest = toml::from_str(text)?;
    check_ps3(&manifest.observe, &manifest.ps3)?;
    Ok(manifest)
}

/// Load the console runner's view of a manifest from a TOML file.
pub fn load_console(path: &Path) -> Result<ConsoleManifest, ManifestError> {
    let text = std::fs::read_to_string(path)?;
    parse_console(&text)
}

/// Parse the console runner's view of a manifest from a TOML string.
pub fn parse_console(text: &str) -> Result<ConsoleManifest, ManifestError> {
    let manifest: ConsoleManifest = toml::from_str(text)?;
    check_ps3(&manifest.observe, &manifest.ps3)?;
    Ok(manifest)
}

#[cfg(test)]
#[path = "tests/manifest_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/ps3_section_tests.rs"]
mod ps3_section_tests;
