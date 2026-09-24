//! The table candidates a discovered index shape admits.

use std::collections::BTreeSet;

use crate::loader::{file_offset_at, LoadSegment};

use super::code_scan::{constant_return, IndexShape};
use super::read::{descriptor_at, read_u64_at};

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
            let mut unique = BTreeSet::new();
            unique.insert(first_ptr);
            let mut entry_zero_references = 1usize;
            let mut descriptor_entries = 1usize;
            let mut zero_environments = usize::from(first.env == 0);
            let mut last_pointer = first_ptr;
            let mut valid = true;
            for index in 1..shape.entry_count {
                let Some(address) = table_vaddr.checked_add((index * shape.entry_width) as u64)
                else {
                    valid = false;
                    break;
                };
                let Some(pointer) = read_u64_at(elf, segments, address) else {
                    valid = false;
                    break;
                };
                if pointer == 0 {
                    last_pointer = pointer;
                    continue;
                }
                let Some(descriptor) = descriptor_at(elf, segments, pointer) else {
                    valid = false;
                    break;
                };
                if descriptor.toc != first.toc || descriptor.env != 0 {
                    valid = false;
                    break;
                }
                descriptor_entries += 1;
                zero_environments += 1;
                if pointer == first_ptr {
                    entry_zero_references += 1;
                }
                last_pointer = pointer;
                unique.insert(pointer);
            }
            if !valid || first.env != 0 || last_pointer != first_ptr {
                continue;
            }
            let Some(file_offset) = file_offset_at(segments, table_vaddr, table_bytes) else {
                continue;
            };
            out.push(TableCandidate {
                table_vaddr,
                table_file_offset: file_offset as u64,
                toc: first.toc,
                unique_descriptors: unique.len(),
                descriptor_entries,
                zero_environments,
                entry_zero_return: Some(entry_zero_return),
                entry_zero_references,
                post_table_zero: post.map(|word| word == 0),
            });
        }
    }
    out
}
