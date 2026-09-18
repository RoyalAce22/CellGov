//! Classifies discovered LV2 syscall slots as implemented, stub, or absent.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::lv2::errno;

use crate::loader::pt_load_segments;
use crate::lv2_table::{self, Lv2TableDiscovery, Lv2TableDiscoveryError};

const MINIMUM_MODE_DIVISOR: usize = 4;
const MINIMUM_DOMINANCE_FACTOR: usize = 4;

/// Classification of one LV2 dispatch-table slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2OrdinalClass {
    /// The table names a non-stub implementation.
    Implemented,
    /// The table names a verified constant-error stub.
    Stub,
    /// The table entry is zero.
    Absent,
}

impl Lv2OrdinalClass {
    /// Returns the stable archive label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::Stub => "stub",
            Self::Absent => "absent",
        }
    }
}

/// One dispatch target verified as a constant-error stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv2StubTarget {
    /// Gives the function-descriptor address stored in the table.
    pub descriptor: u64,
    /// Gives the descriptor's code address.
    pub code: u64,
    /// Gives the returned Cell error code.
    pub errno: u32,
    /// Names the returned Cell error code.
    pub errno_symbol: &'static str,
    /// Counts table entries that name this descriptor.
    pub references: usize,
}

/// One classified syscall-table ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv2ClassifiedOrdinal {
    /// Gives the zero-based table ordinal.
    pub ordinal: usize,
    /// Gives the classification.
    pub class: Lv2OrdinalClass,
    /// Gives the descriptor address, or `None` when absent.
    pub descriptor: Option<u64>,
    /// Gives the code address, or `None` when absent.
    pub code: Option<u64>,
}

/// Histogram evidence that supports the shared-stub decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv2StubEvidence {
    /// Counts distinct nonzero descriptor targets.
    pub descriptor_targets: usize,
    /// Counts references to the histogram mode.
    pub mode_references: usize,
    /// Counts references to the runner-up target.
    pub runner_up_references: usize,
    /// Gives the minimum count required for a mode.
    pub minimum_mode_references: usize,
    /// Gives the required mode-to-runner-up ratio.
    pub minimum_dominance_factor: usize,
}

/// Per-kernel LV2 stub classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lv2StubClassification {
    /// Carries the table discovery and its evidence.
    pub discovery: Lv2TableDiscovery,
    /// Gives the dominant shared-stub target.
    pub primary_stub: Lv2StubTarget,
    /// Lists every repeated or dedicated constant-error target.
    pub stub_targets: Vec<Lv2StubTarget>,
    /// Lists every ordinal in ascending order.
    pub ordinals: Vec<Lv2ClassifiedOrdinal>,
    /// Counts implemented ordinals.
    pub implemented: usize,
    /// Counts stub ordinals.
    pub stub: usize,
    /// Counts absent ordinals.
    pub absent: usize,
    /// Carries the mode and dominance measurements.
    pub evidence: Lv2StubEvidence,
}

/// Why stub classification refused a kernel ELF.
#[derive(Debug, thiserror::Error)]
pub enum Lv2StubClassificationError {
    /// Table discovery failed.
    #[error("LV2 stub classification: {0}")]
    Discovery(#[from] Lv2TableDiscoveryError),
    /// The descriptor histogram has no mode with a wide margin.
    #[error(
        "LV2 stub classification: no clear mode (top={top}, runner_up={runner_up}, minimum={minimum}, factor={factor})"
    )]
    NoClearMode {
        /// Gives the highest reference count.
        top: usize,
        /// Gives the second-highest reference count.
        runner_up: usize,
        /// Gives the minimum accepted mode count.
        minimum: usize,
        /// Gives the required dominance factor.
        factor: usize,
    },
    /// The dominant target does not return one known Cell error.
    #[error(
        "LV2 stub classification: dominant descriptor 0x{descriptor:016x} is not a constant Cell-error leaf"
    )]
    ModeNotConstantError {
        /// Gives the dominant descriptor address.
        descriptor: u64,
    },
}

/// Discover and classify one decrypted LV2 kernel ELF.
///
/// # Errors
///
/// Returns [`Lv2StubClassificationError`] for one of these conditions:
///
/// - Table discovery fails.
/// - The descriptor histogram has no dominant mode.
/// - The dominant mode is not a constant-error leaf.
pub fn classify(elf: &[u8]) -> Result<Lv2StubClassification, Lv2StubClassificationError> {
    let discovery = lv2_table::discover(elf)?;
    classify_discovered(elf, discovery)
}

/// Classify a table that the caller already discovered.
///
/// # Errors
///
/// Returns [`Lv2StubClassificationError`] for one of these conditions:
///
/// - The table changed after discovery.
/// - The descriptor histogram has no dominant mode.
/// - The dominant mode is not a constant-error leaf.
pub fn classify_discovered(
    elf: &[u8],
    discovery: Lv2TableDiscovery,
) -> Result<Lv2StubClassification, Lv2StubClassificationError> {
    let entries = lv2_table::table_entries(elf, &discovery)?;
    let mut histogram: BTreeMap<u64, (u64, usize)> = BTreeMap::new();
    for entry in &entries {
        if let (Some(descriptor), Some(code)) = (entry.descriptor, entry.code) {
            let row = histogram.entry(descriptor).or_insert((code, 0));
            row.1 += 1;
        }
    }
    let mut ranked: Vec<(u64, u64, usize)> = histogram
        .iter()
        .map(|(descriptor, (code, references))| (*descriptor, *code, *references))
        .collect();
    ranked.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    let (mode_descriptor, _mode_code, mode_references) =
        ranked.first().copied().unwrap_or((0, 0, 0));
    let runner_up_references = ranked.get(1).map_or(0, |row| row.2);
    // Absent ordinals are not observations in the descriptor histogram.
    let descriptor_entries = entries
        .iter()
        .filter(|entry| entry.descriptor.is_some())
        .count();
    let minimum_mode_references = descriptor_entries.div_ceil(MINIMUM_MODE_DIVISOR);
    let dominates = runner_up_references == 0
        || mode_references >= runner_up_references.saturating_mul(MINIMUM_DOMINANCE_FACTOR);
    if mode_references < minimum_mode_references || !dominates {
        return Err(Lv2StubClassificationError::NoClearMode {
            top: mode_references,
            runner_up: runner_up_references,
            minimum: minimum_mode_references,
            factor: MINIMUM_DOMINANCE_FACTOR,
        });
    }

    let segments = pt_load_segments(elf).map_err(Lv2TableDiscoveryError::from)?;
    let mut stub_targets = Vec::new();
    for (descriptor, code, references) in &ranked {
        let Some(returned) = lv2_table::constant_return(elf, &segments, *code) else {
            continue;
        };
        let Some(error) = errno::lookup(returned) else {
            continue;
        };
        stub_targets.push(Lv2StubTarget {
            descriptor: *descriptor,
            code: *code,
            errno: returned,
            errno_symbol: error.symbol,
            references: *references,
        });
    }
    stub_targets.sort_by_key(|stub| stub.descriptor);
    let primary_stub = stub_targets
        .iter()
        .copied()
        .find(|stub| stub.descriptor == mode_descriptor)
        .ok_or(Lv2StubClassificationError::ModeNotConstantError {
            descriptor: mode_descriptor,
        })?;
    let stub_descriptors: BTreeSet<u64> = stub_targets.iter().map(|stub| stub.descriptor).collect();

    let mut implemented = 0usize;
    let mut stub = 0usize;
    let mut absent = 0usize;
    let ordinals = entries
        .into_iter()
        .map(|entry| {
            let class = match entry.descriptor {
                None => {
                    absent += 1;
                    Lv2OrdinalClass::Absent
                }
                Some(descriptor) if stub_descriptors.contains(&descriptor) => {
                    stub += 1;
                    Lv2OrdinalClass::Stub
                }
                Some(_) => {
                    implemented += 1;
                    Lv2OrdinalClass::Implemented
                }
            };
            Lv2ClassifiedOrdinal {
                ordinal: entry.ordinal,
                class,
                descriptor: entry.descriptor,
                code: entry.code,
            }
        })
        .collect();

    Ok(Lv2StubClassification {
        discovery,
        primary_stub,
        stub_targets,
        ordinals,
        implemented,
        stub,
        absent,
        evidence: Lv2StubEvidence {
            descriptor_targets: histogram.len(),
            mode_references,
            runner_up_references,
            minimum_mode_references,
            minimum_dominance_factor: MINIMUM_DOMINANCE_FACTOR,
        },
    })
}

#[cfg(test)]
#[path = "tests/lv2_stub_tests.rs"]
mod tests;
