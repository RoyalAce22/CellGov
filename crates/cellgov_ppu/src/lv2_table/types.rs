//! The discovery result, its evidence and confidence, the table entries, and the refusals.

use crate::loader::LoadError;

/// Names the evidence path that produced a table discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2DiscoveryMethod {
    /// Uses the `sc` vector's index shape to find one descriptor-pointer array.
    ScVectorDescriptorArray,
}

impl Lv2DiscoveryMethod {
    /// Returns the stable label used in reports and archive rows.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ScVectorDescriptorArray => "sc_vector_descriptor_array",
        }
    }
}

/// States whether structural evidence uniquely identifies one table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2DiscoveryConfidence {
    /// Every structural check uniquely identifies the same table.
    High,
}

impl Lv2DiscoveryConfidence {
    /// Returns the stable label used in reports and archive rows.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
        }
    }
}

/// Describes how one dispatch-table entry names its handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2TableEntryFormat {
    /// Stores a 64-bit pointer to a three-doubleword PPC64 function descriptor.
    Ppc64DescriptorPointer,
}

impl Lv2TableEntryFormat {
    /// Returns the stable label used in reports and archive rows.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ppc64DescriptorPointer => "ppc64_descriptor_pointer",
        }
    }
}

/// Records structural checks that support one discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv2DiscoveryEvidence {
    /// Counts executable addresses that the `sc` vector materializes.
    pub vector_targets: usize,
    /// Counts handler targets whose code contains the index sequence.
    pub handler_matches: usize,
    /// Counts table-shaped arrays that pass every descriptor check.
    pub table_candidates: usize,
    /// Counts table entries that name valid descriptors.
    pub descriptor_entries: usize,
    /// Counts distinct descriptors that the table names.
    pub unique_descriptors: usize,
    /// Counts entries that point to the entry-zero descriptor.
    pub entry_zero_references: usize,
    /// Reports whether the final slot points to the entry-zero descriptor.
    pub last_entry_is_entry_zero: bool,
    /// Counts descriptor entries with a zero environment word.
    pub zero_environments: usize,
    /// Reports whether every descriptor carries the same TOC.
    pub consistent_toc: bool,
    /// Reports whether the first word after the table is zero.
    ///
    /// `None` means that the table ends at the file-backed segment boundary.
    pub post_table_zero: Option<bool>,
    /// Contains the low 32 bits that entry zero returns when it is a three-instruction leaf.
    pub entry_zero_return: Option<u32>,
}

/// Describes one high-confidence LV2 dispatch-table discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv2TableDiscovery {
    /// Names the discovery method.
    pub method: Lv2DiscoveryMethod,
    /// Gives the confidence level.
    pub confidence: Lv2DiscoveryConfidence,
    /// Gives the effective address of the architectural System Call vector.
    pub vector_vaddr: u64,
    /// Gives the effective address of the handler that contains the table index.
    pub handler_vaddr: u64,
    /// Gives the effective address of the table.
    pub table_vaddr: u64,
    /// Gives the file offset of the table inside the ELF.
    pub table_file_offset: u64,
    /// Gives the table entry count recovered from the handler bound.
    pub entry_count: usize,
    /// Gives the entry width recovered from the handler shift.
    pub entry_width: usize,
    /// Names the table entry representation.
    pub entry_format: Lv2TableEntryFormat,
    /// Gives the TOC that every referenced function descriptor shares.
    pub toc: u64,
    /// Evidence that supports the confidence.
    pub evidence: Lv2DiscoveryEvidence,
}

/// One ordinal read from a discovered dispatch table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lv2DispatchEntry {
    /// Zero-based table ordinal.
    pub(crate) ordinal: usize,
    /// Function-descriptor address, or `None` for an absent slot.
    pub(crate) descriptor: Option<u64>,
    /// Descriptor code address, or `None` for an absent slot.
    pub(crate) code: Option<u64>,
}

/// Reports why an ELF did not produce one unique, high-confidence table.
#[derive(Debug, thiserror::Error)]
pub enum Lv2TableDiscoveryError {
    /// The ELF has malformed load segments.
    #[error("LV2 table discovery: {0}")]
    Elf(#[from] LoadError),
    /// A load segment lies outside the input or violates ELF bounds.
    #[error("LV2 table discovery: malformed PT_LOAD segment {index}")]
    MalformedSegment {
        /// Gives the program-header index.
        index: usize,
    },
    /// The ELF has no executable load segment.
    #[error("LV2 table discovery: ELF has no executable PT_LOAD")]
    NoExecutableSegment,
    /// The System Call vector is absent or names no executable handler.
    #[error("LV2 table discovery: System Call vector yields no indexed handler")]
    HandlerNotFound,
    /// Several plausible table-index sequences remain.
    #[error("LV2 table discovery: {count} plausible handler index sequences remain")]
    AmbiguousHandler {
        /// Gives the plausible sequence count.
        count: usize,
    },
    /// No descriptor-pointer array matches the handler's bound and stride.
    #[error(
        "LV2 table discovery: no table matches handler shape count={entry_count} width={entry_width}"
    )]
    NoTableCandidate {
        /// Gives the entry count recovered from the handler.
        entry_count: usize,
        /// Gives the entry width recovered from the handler.
        entry_width: usize,
    },
    /// Several arrays satisfy every structural check.
    #[error("LV2 table discovery: {count} table candidates remain; refusing ambiguity")]
    AmbiguousTable {
        /// Gives the fully valid candidate count.
        count: usize,
    },
    /// A table entry changed or became malformed after discovery.
    #[error("LV2 table discovery: ordinal {ordinal} is not a valid descriptor pointer")]
    InvalidTableEntry {
        /// Gives the invalid ordinal.
        ordinal: usize,
    },
}
