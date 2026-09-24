//! The program-header scan and the vaddr-to-file-offset segment map a PRX parse reads through.

use cellgov_ps3_abi::format::elf::{PT_LOAD, PT_PRX_RELOC};

use crate::loader;

use super::parse::PrxParseError;

pub(super) struct RawPhdr {
    #[allow(dead_code)]
    p_type: u32,
    pub(super) p_offset: usize,
    pub(super) p_vaddr: u64,
    pub(super) p_paddr: u64,
    pub(super) p_filesz: u64,
    pub(super) p_memsz: u64,
}

/// Every PT_LOAD in program-header order, plus the PT_PRX_RELOC segment.
pub(super) fn scan_phdrs(data: &[u8]) -> Result<(Vec<RawPhdr>, Option<RawPhdr>), PrxParseError> {
    let phoff = loader::read_u64(data, 32) as usize;
    let phentsize = loader::read_u16(data, 54) as usize;
    let phnum = loader::read_u16(data, 56) as usize;
    // ELF64 phdr is 56 bytes; smaller phentsize would alias entries.
    if phentsize < 56 {
        return Err(PrxParseError::OutOfBounds);
    }

    let mut loads: Vec<RawPhdr> = Vec::new();
    let mut reloc_phdr: Option<RawPhdr> = None;

    for i in 0..phnum {
        let base = i
            .checked_mul(phentsize)
            .and_then(|off| phoff.checked_add(off))
            .ok_or(PrxParseError::OutOfBounds)?;
        let end = base
            .checked_add(phentsize)
            .ok_or(PrxParseError::OutOfBounds)?;
        if end > data.len() {
            return Err(PrxParseError::OutOfBounds);
        }
        let p_type = loader::read_u32(data, base);
        let phdr = RawPhdr {
            p_type,
            p_offset: loader::read_u64(data, base + 8) as usize,
            p_vaddr: loader::read_u64(data, base + 16),
            p_paddr: loader::read_u64(data, base + 24),
            p_filesz: loader::read_u64(data, base + 32),
            p_memsz: loader::read_u64(data, base + 40),
        };

        match p_type {
            PT_LOAD => loads.push(phdr),
            PT_PRX_RELOC => reloc_phdr = Some(phdr),
            _ => {}
        }
    }

    Ok((loads, reloc_phdr))
}

pub(super) struct SegEntry {
    pub(super) vaddr: usize,
    pub(super) file_offset: usize,
    pub(super) size: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct VaddrRange {
    pub(super) start: u32,
    pub(super) end: u32,
}

pub(super) fn v2f(seg_map: &[SegEntry], vaddr: usize) -> Option<usize> {
    for seg in seg_map {
        if vaddr >= seg.vaddr && vaddr < seg.vaddr + seg.size {
            return Some(vaddr - seg.vaddr + seg.file_offset);
        }
    }
    None
}
