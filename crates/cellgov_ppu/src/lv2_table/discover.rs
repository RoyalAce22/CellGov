//! Discovering the dispatch table, and reading its entries back.

use cellgov_ps3_abi::hw::ppu::SYSTEM_CALL_VECTOR_OFFSET;

use crate::loader::{pt_load_segments, LoadSegment};

use super::candidates::scan_candidates;
use super::code_scan::{index_shapes, materialized_addresses, IndexShape};
use super::read::{descriptor_at, is_executable_address, read_u64_at, words_at};
use super::types::{
    Lv2DiscoveryConfidence, Lv2DiscoveryEvidence, Lv2DiscoveryMethod, Lv2DispatchEntry,
    Lv2TableDiscovery, Lv2TableDiscoveryError, Lv2TableEntryFormat,
};

const VECTOR_WORDS: usize = 64;
const HANDLER_WORDS: usize = 512;
pub(super) const MAX_DISCOVERED_SLOTS: usize = 1 << 16;

/// Discover the LV2 dispatch table in a decrypted kernel ELF.
///
/// Discovery returns only a unique, high-confidence table.
///
/// # Errors
///
/// Returns [`Lv2TableDiscoveryError`] if discovery cannot produce one unique,
/// high-confidence table.
pub fn discover(elf: &[u8]) -> Result<Lv2TableDiscovery, Lv2TableDiscoveryError> {
    let segments = pt_load_segments(elf)?;
    validate_segments(elf, &segments)?;
    let image_base = segments
        .iter()
        .filter(|segment| segment.executable)
        .map(|segment| segment.vaddr)
        .min()
        .ok_or(Lv2TableDiscoveryError::NoExecutableSegment)?;
    let vector_vaddr = image_base
        .checked_add(SYSTEM_CALL_VECTOR_OFFSET)
        .ok_or(Lv2TableDiscoveryError::HandlerNotFound)?;
    let vector_words = words_at(elf, &segments, vector_vaddr, VECTOR_WORDS)
        .ok_or(Lv2TableDiscoveryError::HandlerNotFound)?;
    let vector_targets: Vec<_> = materialized_addresses(&vector_words)
        .into_iter()
        .filter(|target| is_executable_address(&segments, *target))
        .collect();

    let mut shapes = Vec::new();
    for target in &vector_targets {
        let Some(words) = words_at(elf, &segments, *target, HANDLER_WORDS) else {
            continue;
        };
        for (entry_count, entry_width) in index_shapes(&words, *target) {
            shapes.push(IndexShape {
                handler_vaddr: *target,
                entry_count,
                entry_width,
            });
        }
    }
    shapes.sort_by_key(|shape| (shape.handler_vaddr, shape.entry_count, shape.entry_width));
    let shape = match shapes.as_slice() {
        [] => return Err(Lv2TableDiscoveryError::HandlerNotFound),
        [shape] => *shape,
        many => {
            return Err(Lv2TableDiscoveryError::AmbiguousHandler { count: many.len() });
        }
    };

    let candidates = scan_candidates(elf, &segments, shape);
    let candidate = match candidates.as_slice() {
        [] => {
            return Err(Lv2TableDiscoveryError::NoTableCandidate {
                entry_count: shape.entry_count,
                entry_width: shape.entry_width,
            });
        }
        [candidate] => *candidate,
        many => {
            return Err(Lv2TableDiscoveryError::AmbiguousTable { count: many.len() });
        }
    };

    Ok(Lv2TableDiscovery {
        method: Lv2DiscoveryMethod::ScVectorDescriptorArray,
        confidence: Lv2DiscoveryConfidence::High,
        vector_vaddr,
        handler_vaddr: shape.handler_vaddr,
        table_vaddr: candidate.table_vaddr,
        table_file_offset: candidate.table_file_offset,
        entry_count: shape.entry_count,
        entry_width: shape.entry_width,
        entry_format: Lv2TableEntryFormat::Ppc64DescriptorPointer,
        toc: candidate.toc,
        evidence: Lv2DiscoveryEvidence {
            vector_targets: vector_targets.len(),
            handler_matches: shapes.len(),
            table_candidates: candidates.len(),
            descriptor_entries: candidate.descriptor_entries,
            unique_descriptors: candidate.unique_descriptors,
            entry_zero_references: candidate.entry_zero_references,
            last_entry_is_entry_zero: true,
            zero_environments: candidate.zero_environments,
            consistent_toc: true,
            post_table_zero: candidate.post_table_zero,
            entry_zero_return: candidate.entry_zero_return,
        },
    })
}

/// Validates and returns the entries from a prior discovery.
///
/// # Errors
///
/// Returns [`Lv2TableDiscoveryError::InvalidTableEntry`] if a nonzero
/// entry does not name the descriptor shape that discovery validated.
pub(crate) fn table_entries(
    elf: &[u8],
    discovery: &Lv2TableDiscovery,
) -> Result<Vec<Lv2DispatchEntry>, Lv2TableDiscoveryError> {
    let segments = pt_load_segments(elf)?;
    validate_segments(elf, &segments)?;
    let mut entries = Vec::with_capacity(discovery.entry_count);
    for ordinal in 0..discovery.entry_count {
        let offset = ordinal
            .checked_mul(discovery.entry_width)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or(Lv2TableDiscoveryError::InvalidTableEntry { ordinal })?;
        let address = discovery
            .table_vaddr
            .checked_add(offset)
            .ok_or(Lv2TableDiscoveryError::InvalidTableEntry { ordinal })?;
        let pointer = read_u64_at(elf, &segments, address)
            .ok_or(Lv2TableDiscoveryError::InvalidTableEntry { ordinal })?;
        if pointer == 0 {
            entries.push(Lv2DispatchEntry {
                ordinal,
                descriptor: None,
                code: None,
            });
            continue;
        }
        let descriptor = descriptor_at(elf, &segments, pointer)
            .ok_or(Lv2TableDiscoveryError::InvalidTableEntry { ordinal })?;
        if descriptor.toc != discovery.toc || descriptor.env != 0 {
            return Err(Lv2TableDiscoveryError::InvalidTableEntry { ordinal });
        }
        entries.push(Lv2DispatchEntry {
            ordinal,
            descriptor: Some(pointer),
            code: Some(descriptor.code),
        });
    }
    Ok(entries)
}

fn validate_segments(elf: &[u8], segments: &[LoadSegment]) -> Result<(), Lv2TableDiscoveryError> {
    for segment in segments {
        if segment.filesz > segment.memsz
            || segment
                .file_offset
                .checked_add(segment.filesz)
                .is_none_or(|end| end > elf.len() as u64)
            || segment.vaddr.checked_add(segment.memsz).is_none()
        {
            return Err(Lv2TableDiscoveryError::MalformedSegment {
                index: segment.index,
            });
        }
    }
    Ok(())
}
