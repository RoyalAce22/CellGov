//! The table candidates a discovered index shape admits.

use std::collections::BTreeSet;

use crate::loader::{file_offset_at, LoadSegment};

use super::code_scan::{constant_return, IndexShape};
use super::read::{descriptor_at, read_u64_at, Descriptor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TableCandidate {
    pub(super) table_vaddr: u64,
    pub(super) table_file_offset: u64,
    pub(super) toc: u64,
    pub(super) unique_descriptors: usize,
    pub(super) descriptor_entries: usize,
    pub(super) zero_environments: usize,
    pub(super) entry_zero_return: Option<u32>,
    pub(super) entry_zero_references: usize,
    pub(super) post_table_zero: Option<bool>,
}

pub(super) fn scan_candidates(
    elf: &[u8],
    segments: &[LoadSegment],
    shape: IndexShape,
) -> Vec<TableCandidate> {
    let Some(table_bytes) = shape.entry_count.checked_mul(shape.entry_width) else {
        return Vec::new();
    };
    if shape.entry_width != core::mem::size_of::<u64>() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for segment in segments.iter().filter(|segment| segment.readable) {
        let Ok(filesz) = usize::try_from(segment.filesz) else {
            continue;
        };
        let Some(limit) = filesz.checked_sub(table_bytes) else {
            continue;
        };
        let alignment = shape.entry_width as u64;
        let skip = ((alignment - segment.vaddr % alignment) % alignment) as usize;
        for relative in (skip..=limit).step_by(shape.entry_width) {
            let table_vaddr = segment.vaddr + relative as u64;
            let post = read_u64_at(elf, segments, table_vaddr + table_bytes as u64);
            let Some(first_ptr) = read_u64_at(elf, segments, table_vaddr) else {
                continue;
            };
            let Some(first) = descriptor_at(elf, segments, first_ptr) else {
                continue;
            };
            let Some(entry_zero_return) = constant_return(elf, segments, first.code) else {
                continue;
            };
            let Some(stats) = walk_table(elf, segments, table_vaddr, shape, first_ptr, first)
            else {
                continue;
            };
            if first.env != 0 || stats.last_pointer != first_ptr {
                continue;
            }
            let Some(file_offset) = file_offset_at(segments, table_vaddr, table_bytes) else {
                continue;
            };
            out.push(TableCandidate {
                table_vaddr,
                table_file_offset: file_offset as u64,
                toc: first.toc,
                unique_descriptors: stats.unique_descriptors,
                descriptor_entries: stats.descriptor_entries,
                zero_environments: stats.zero_environments,
                entry_zero_return: Some(entry_zero_return),
                entry_zero_references: stats.entry_zero_references,
                post_table_zero: post.map(|word| word == 0),
            });
        }
    }
    out
}

/// What [`walk_table`] counts over one candidate table.
struct TableStats {
    unique_descriptors: usize,
    descriptor_entries: usize,
    zero_environments: usize,
    entry_zero_references: usize,
    last_pointer: u64,
}

/// Walks entries 1.. of the table at `table_vaddr` whose entry 0 is
/// `first_ptr` -> `first`.
///
/// `None` when an entry address overflows or does not read, or when a
/// nonzero entry names no descriptor or one with a different TOC or a
/// nonzero environment.
fn walk_table(
    elf: &[u8],
    segments: &[LoadSegment],
    table_vaddr: u64,
    shape: IndexShape,
    first_ptr: u64,
    first: Descriptor,
) -> Option<TableStats> {
    let mut unique = BTreeSet::new();
    unique.insert(first_ptr);
    let mut stats = TableStats {
        unique_descriptors: 0,
        descriptor_entries: 1,
        zero_environments: usize::from(first.env == 0),
        entry_zero_references: 1,
        last_pointer: first_ptr,
    };
    for index in 1..shape.entry_count {
        let address = table_vaddr.checked_add((index * shape.entry_width) as u64)?;
        let pointer = read_u64_at(elf, segments, address)?;
        if pointer == 0 {
            stats.last_pointer = pointer;
            continue;
        }
        let descriptor = descriptor_at(elf, segments, pointer)?;
        if descriptor.toc != first.toc || descriptor.env != 0 {
            return None;
        }
        stats.descriptor_entries += 1;
        stats.zero_environments += 1;
        if pointer == first_ptr {
            stats.entry_zero_references += 1;
        }
        stats.last_pointer = pointer;
        unique.insert(pointer);
    }
    stats.unique_descriptors = unique.len();
    Some(stats)
}
