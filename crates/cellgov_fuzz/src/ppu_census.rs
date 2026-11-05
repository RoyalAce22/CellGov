//! A count of PPU instruction words by architectural unit, and by what
//! CellGov does with each word: execute it, refuse it, or not
//! recognize it.
//!
//! The units are the reference units of [`crate::ppu_reference`]. A
//! census of a title's code is the measure of which units the title
//! exercises.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ppu::decode::decode;
use cellgov_ppu::instruction::fuzz::PpuOutcomeClass;
use cellgov_ppu::instruction::PpuDecodeError;
use cellgov_ppu::prescan::{elf_text_ranges, PrescanError};
use cellgov_ppu::PpuFaultClass;
use cellgov_ps3_abi::format::elf::{
    ELFCLASS64, ELF_EI_CLASS, ELF_E_MACHINE_OFFSET, ELF_MAGIC, EM_PPC64,
};
use serde::{Deserialize, Serialize};

use crate::ppu_reference::{decoded_word_unit, unit_key, PpuReferenceUnit};

/// Version of [`PpuCensusRecord`].
pub const PPU_CENSUS_SCHEMA_VERSION: u32 = 1;

/// What CellGov does with one word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuWordFate {
    /// The word decodes, and some execution of it does not fault.
    Implemented,
    /// The word decodes, and every execution of it faults with this
    /// class.
    Refused(PpuFaultClass),
    /// The decoder rejects the word.
    NotRecognized,
}

/// The unit a word belongs to, and what CellGov does with it.
///
/// A word the decoder rejects as a documented encoding with no decoder
/// arm belongs to the unit of the mnemonic the rejection names. A word
/// that no documented encoding matches belongs to
/// [`PpuReferenceUnit::UnassignedOpcodes`].
pub fn classify_word(raw: u32) -> (PpuReferenceUnit, PpuWordFate) {
    let instruction = match decode(raw) {
        Ok(instruction) => instruction,
        Err(PpuDecodeError::DecoderArmUnimplemented { mnemonic, .. }) => {
            let unit = PpuReferenceUnit::Instruction {
                mnemonic: mnemonic.into(),
            };
            return (unit, PpuWordFate::NotRecognized);
        }
        Err(PpuDecodeError::EncodingNotRecognized { .. }) => {
            return (
                PpuReferenceUnit::UnassignedOpcodes,
                PpuWordFate::NotRecognized,
            );
        }
    };
    let descriptor = instruction.fuzz_descriptor(raw);
    let unit = decoded_word_unit(raw, descriptor.kind);
    let fate = match (descriptor.outcomes, descriptor.faults) {
        ([PpuOutcomeClass::Fault], [class]) => PpuWordFate::Refused(*class),
        _ => PpuWordFate::Implemented,
    };
    (unit, fate)
}

/// The name a census writes for a refusal's fault class.
pub fn refusal_name(class: PpuFaultClass) -> &'static str {
    match class {
        PpuFaultClass::InstructionStorage => "instruction_storage",
        PpuFaultClass::InstructionSegment => "instruction_segment",
        PpuFaultClass::DataStorage => "data_storage",
        PpuFaultClass::DataSegment => "data_segment",
        PpuFaultClass::Alignment => "alignment",
        PpuFaultClass::ProgramFpEnabled => "program_fp_enabled",
        PpuFaultClass::ProgramIllegal => "illegal",
        PpuFaultClass::ProgramPrivileged => "privileged",
        PpuFaultClass::ProgramTrap => "trap",
        PpuFaultClass::FpUnavailable => "fp_unavailable",
        PpuFaultClass::VxuUnavailable => "vxu_unavailable",
        PpuFaultClass::SystemCall => "system_call",
        PpuFaultClass::Unimplemented => "unimplemented",
        PpuFaultClass::DebugBreak => "debug_break",
        PpuFaultClass::Internal => "internal",
    }
}

/// One unit's word counts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PpuCensusCounts {
    /// Words CellGov executes.
    pub implemented: u64,
    /// Words CellGov refuses, by [`refusal_name`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refused: BTreeMap<String, u64>,
    /// Words the decoder rejects.
    pub not_recognized: u64,
}

impl PpuCensusCounts {
    /// Every word counted.
    pub fn seen(&self) -> u64 {
        self.implemented + self.refused.values().sum::<u64>() + self.not_recognized
    }

    fn add(&mut self, fate: PpuWordFate) {
        match fate {
            PpuWordFate::Implemented => self.implemented += 1,
            PpuWordFate::Refused(class) => {
                *self.refused.entry(refusal_name(class).into()).or_default() += 1;
            }
            PpuWordFate::NotRecognized => self.not_recognized += 1,
        }
    }

    fn merge(&mut self, other: &Self) {
        self.implemented += other.implemented;
        for (class, words) in &other.refused {
            *self.refused.entry(class.clone()).or_default() += words;
        }
        self.not_recognized += other.not_recognized;
    }
}

/// Words counted by unit key ([`unit_key`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PpuCensus {
    units: BTreeMap<String, PpuCensusCounts>,
}

impl PpuCensus {
    /// An empty census.
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one word.
    pub fn add(&mut self, raw: u32) {
        let (unit, fate) = classify_word(raw);
        self.units.entry(unit_key(&unit)).or_default().add(fate);
    }

    /// Count the words of the executable text of the PPU ELF `elf`, the
    /// ranges [`elf_text_ranges`] names. Returns the words counted.
    ///
    /// # Errors
    ///
    /// The [`PrescanError`] of an ELF the PPU loader cannot read.
    pub fn add_elf(&mut self, elf: &[u8]) -> Result<u64, PrescanError> {
        let (ranges, _) = elf_text_ranges(elf)?;
        let before = self.words();
        for (lo, hi) in ranges {
            for word in elf[lo..hi].chunks_exact(4) {
                self.add(u32::from_be_bytes([word[0], word[1], word[2], word[3]]));
            }
        }
        Ok(self.words() - before)
    }

    /// Count `elf` as [`Self::add_elf`] does, once for each distinct image.
    ///
    /// The call adds the FNV-1a hash of `elf` to `seen` only after the
    /// count succeeds, so an unreadable image gets the same refusal for
    /// each file that holds it. Returns the words counted, or `None` when
    /// `elf` is no PPU ELF or `seen` holds its hash.
    ///
    /// # Errors
    ///
    /// As [`Self::add_elf`].
    pub fn add_new_elf(
        &mut self,
        elf: &[u8],
        seen: &mut BTreeSet<u64>,
    ) -> Result<Option<u64>, PrescanError> {
        let hash = cellgov_mem::fnv1a(elf);
        if !is_ppu_elf(elf) || seen.contains(&hash) {
            return Ok(None);
        }
        let words = self.add_elf(elf)?;
        seen.insert(hash);
        Ok(Some(words))
    }

    /// Add every count of `other`.
    pub fn merge(&mut self, other: &Self) {
        for (unit, counts) in &other.units {
            self.units.entry(unit.clone()).or_default().merge(counts);
        }
    }

    /// Each unit with at least one word, in unit-key order.
    pub fn units(&self) -> &BTreeMap<String, PpuCensusCounts> {
        &self.units
    }

    /// Every word counted.
    pub fn words(&self) -> u64 {
        self.units.values().map(PpuCensusCounts::seen).sum()
    }
}

/// Whether `bytes` is a 64-bit PowerPC ELF.
fn is_ppu_elf(bytes: &[u8]) -> bool {
    let machine = bytes
        .get(ELF_E_MACHINE_OFFSET..ELF_E_MACHINE_OFFSET + 2)
        .map(|half| u16::from_be_bytes([half[0], half[1]]));
    bytes.starts_with(&ELF_MAGIC)
        && bytes.get(ELF_EI_CLASS) == Some(&ELFCLASS64)
        && machine == Some(EM_PPC64)
}

/// One title's line in a [`PpuCensusRecord`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PpuCensusTitle {
    /// The title's content id.
    pub content_id: String,
    /// Why nothing of the title was read, if nothing was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    /// PPU images first found in this title.
    pub images: usize,
    /// Words those images hold.
    pub words: u64,
    /// Files that could not be read.
    pub unread: usize,
}

/// A census of the installed titles' PPU code, as a file records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PpuCensusRecord {
    /// [`PPU_CENSUS_SCHEMA_VERSION`].
    pub schema: u32,
    /// Each title read, in the order read.
    pub titles: Vec<PpuCensusTitle>,
    /// The counts by unit key.
    pub units: BTreeMap<String, PpuCensusCounts>,
}

impl PpuCensusRecord {
    /// The record of `census` over `titles`.
    pub fn new(titles: Vec<PpuCensusTitle>, census: &PpuCensus) -> Self {
        Self {
            schema: PPU_CENSUS_SCHEMA_VERSION,
            titles,
            units: census.units.clone(),
        }
    }
}

/// A census record that does not parse or has another schema.
#[derive(Debug, thiserror::Error)]
pub enum PpuCensusRecordError {
    /// The text is not a census record.
    #[error("PPU census record: {0}")]
    Parse(#[from] serde_json::Error),
    /// The record has a schema this build does not read.
    #[error("PPU census record schema {found}; this build reads {PPU_CENSUS_SCHEMA_VERSION}")]
    Schema {
        /// The record's schema.
        found: u32,
    },
}

/// Parse a census record.
///
/// # Errors
///
/// Text that is no census record, or one of another schema.
pub fn parse_census_record(text: &str) -> Result<PpuCensusRecord, PpuCensusRecordError> {
    let record: PpuCensusRecord = serde_json::from_str(text)?;
    if record.schema != PPU_CENSUS_SCHEMA_VERSION {
        return Err(PpuCensusRecordError::Schema {
            found: record.schema,
        });
    }
    Ok(record)
}

#[cfg(test)]
#[path = "tests/ppu_census_tests.rs"]
mod tests;
