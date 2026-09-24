//! Reading words, doublewords and function descriptors out of the loaded segments.

use cellgov_ps3_abi::format::elf::ppc64_function_descriptor;

use crate::loader::{file_offset_at, LoadSegment};

#[derive(Debug, Clone, Copy)]
pub(super) struct Descriptor {
    pub(super) code: u64,
    pub(super) toc: u64,
    pub(super) env: u64,
}

pub(super) fn descriptor_at(
    elf: &[u8],
    segments: &[LoadSegment],
    address: u64,
) -> Option<Descriptor> {
    if !address.is_multiple_of(core::mem::size_of::<u64>() as u64) {
        return None;
    }
    let offset = file_offset_at(segments, address, ppc64_function_descriptor::SIZE)?;
    let code = read_be_u64(elf, offset + ppc64_function_descriptor::CODE_OFFSET)?;
    let toc = read_be_u64(elf, offset + ppc64_function_descriptor::TOC_OFFSET)?;
    let env = read_be_u64(elf, offset + ppc64_function_descriptor::ENV_OFFSET)?;
    if code % 4 != 0 || !is_executable_address(segments, code) || !is_mapped_address(segments, toc)
    {
        return None;
    }
    Some(Descriptor { code, toc, env })
}

pub(super) fn words_at(
    elf: &[u8],
    segments: &[LoadSegment],
    address: u64,
    max_words: usize,
) -> Option<Vec<u32>> {
    let segment = segments.iter().find(|segment| {
        segment.executable
            && address >= segment.vaddr
            && address < segment.vaddr.saturating_add(segment.filesz)
    })?;
    let available = segment
        .vaddr
        .checked_add(segment.filesz)?
        .checked_sub(address)? as usize;
    let word_count = max_words.min(available / 4);
    let offset = file_offset_at(segments, address, word_count * 4)?;
    Some(
        elf[offset..offset + word_count * 4]
            .chunks_exact(4)
            .map(|bytes| u32::from_be_bytes(bytes.try_into().expect("four-byte chunk")))
            .collect(),
    )
}

pub(super) fn read_u64_at(elf: &[u8], segments: &[LoadSegment], address: u64) -> Option<u64> {
    let offset = file_offset_at(segments, address, core::mem::size_of::<u64>())?;
    read_be_u64(elf, offset)
}

fn read_be_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    Some(u64::from_be_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

pub(super) fn is_executable_address(segments: &[LoadSegment], address: u64) -> bool {
    segments.iter().any(|segment| {
        segment.executable
            && address >= segment.vaddr
            && address < segment.vaddr.saturating_add(segment.filesz)
    })
}

fn is_mapped_address(segments: &[LoadSegment], address: u64) -> bool {
    segments.iter().any(|segment| {
        address >= segment.vaddr && address < segment.vaddr.saturating_add(segment.memsz)
    })
}
