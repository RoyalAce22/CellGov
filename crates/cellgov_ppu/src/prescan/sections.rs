//! ELF section-header walk used to clip PF_X PT_LOADs down to
//! `SHT_PROGBITS + SHF_ALLOC + SHF_EXECINSTR` sub-ranges.
//!
//! Stripped binaries (`e_shoff == 0`) return an empty range list so
//! the caller falls back to the segment walk. A present-but-malformed
//! section table errors; it is not a fallback trigger.

use cellgov_ps3_abi::elf::{
    ELF64_E_SHENTSIZE, ELF64_E_SHNUM, ELF64_E_SHOFF, ELF64_E_SHSTRNDX, ELF64_SHENT_SIZE,
    ELF64_SH_FLAGS, ELF64_SH_NAME, ELF64_SH_OFFSET, ELF64_SH_SIZE, ELF64_SH_TYPE, ELF_HEADER_SIZE,
    SHF_ALLOC, SHF_EXECINSTR, SHN_UNDEF, SHT_PROGBITS, SHT_STRTAB,
};

use super::error::PrescanError;

// Range guarantee: the caller's runtime bounds checks combine with
// the `const _: () = ...` field-offset assertions in
// `cellgov_ps3_abi::elf` to prove every read in this module is in
// range. A future edit that breaks either coupling fails compilation
// at the const_assert before any read can fall out of range.
use cellgov_mem::be::{read_u16, read_u32, read_u64};

/// The "executable program-bits section" predicate. Shared between
/// [`executable_progbits_ranges`] and [`executable_sections_anonymous`].
fn is_executable_progbits(sh_type: u32, sh_flags: u64, sh_size: u64) -> bool {
    sh_type == SHT_PROGBITS
        && (sh_flags & (SHF_ALLOC | SHF_EXECINSTR)) == (SHF_ALLOC | SHF_EXECINSTR)
        && sh_size != 0
}

/// File-offset range covering all section-header entries that hold
/// executable program bits (`SHT_PROGBITS` with both `SHF_ALLOC`
/// and `SHF_EXECINSTR`), sorted ascending by start and deduped.
///
/// # Errors
///
/// Returns [`PrescanError::MalformedSectionTable`] when the section
/// header table is present (`e_shoff != 0`) but malformed --
/// out-of-range entries, undersized `e_shentsize`, or a section
/// header whose `sh_offset + sh_size` exceeds the file.
///
/// A stripped binary (`e_shoff == 0`) returns `Ok(vec![])`; callers
/// then fall back to the segment walk.
pub fn executable_progbits_ranges(elf_data: &[u8]) -> Result<Vec<(usize, usize)>, PrescanError> {
    if elf_data.len() < ELF_HEADER_SIZE {
        return Ok(Vec::new());
    }

    let e_shoff = read_u64(elf_data, ELF64_E_SHOFF);
    if e_shoff == 0 {
        return Ok(Vec::new());
    }

    let e_shentsize = read_u16(elf_data, ELF64_E_SHENTSIZE);
    let e_shnum = read_u16(elf_data, ELF64_E_SHNUM);

    let shentsize = usize::from(e_shentsize);
    if shentsize < ELF64_SHENT_SIZE {
        return Err(PrescanError::MalformedSectionTable);
    }

    let shnum = usize::from(e_shnum);
    let Ok(shoff) = usize::try_from(e_shoff) else {
        return Err(PrescanError::MalformedSectionTable);
    };

    let mut ranges = Vec::new();
    for i in 0..shnum {
        let base = shoff
            .checked_add(
                i.checked_mul(shentsize)
                    .ok_or(PrescanError::MalformedSectionTable)?,
            )
            .ok_or(PrescanError::MalformedSectionTable)?;
        let end = base
            .checked_add(shentsize)
            .ok_or(PrescanError::MalformedSectionTable)?;
        if end > elf_data.len() {
            return Err(PrescanError::MalformedSectionTable);
        }

        let sh_type = read_u32(elf_data, base + ELF64_SH_TYPE);
        let sh_flags = read_u64(elf_data, base + ELF64_SH_FLAGS);
        let sh_offset = read_u64(elf_data, base + ELF64_SH_OFFSET);
        let sh_size = read_u64(elf_data, base + ELF64_SH_SIZE);

        if !is_executable_progbits(sh_type, sh_flags, sh_size) {
            continue;
        }

        let Ok(lo) = usize::try_from(sh_offset) else {
            return Err(PrescanError::MalformedSectionTable);
        };
        let Ok(sz) = usize::try_from(sh_size) else {
            return Err(PrescanError::MalformedSectionTable);
        };
        let hi = lo
            .checked_add(sz)
            .ok_or(PrescanError::MalformedSectionTable)?;
        if hi > elf_data.len() {
            return Err(PrescanError::MalformedSectionTable);
        }
        ranges.push((lo, hi));
    }

    ranges.sort_unstable();
    ranges.dedup();
    Ok(ranges)
}

/// True when the section table is present but every qualifying
/// executable section has no usable name (`e_shstrndx == SHN_UNDEF`,
/// the indexed `.shstrtab` is missing / wrong type / empty, or every
/// `sh_name` points at a zero byte).
///
/// # Errors
///
/// Returns [`PrescanError::MalformedSectionTable`] when the section
/// table is present but a parser bound fails (undersized
/// `e_shentsize`, out-of-range section / strtab entry, payload past
/// end of file).
///
/// Returns `Ok(false)` when `e_shoff == 0` so the caller's
/// `SegmentFallback` decision in [`super::scan::scan_elf_text`]
/// stays the single source of truth.
pub fn executable_sections_anonymous(elf_data: &[u8]) -> Result<bool, PrescanError> {
    if elf_data.len() < ELF_HEADER_SIZE {
        return Ok(false);
    }

    let e_shoff = read_u64(elf_data, ELF64_E_SHOFF);
    if e_shoff == 0 {
        return Ok(false);
    }

    let e_shentsize = read_u16(elf_data, ELF64_E_SHENTSIZE);
    let e_shnum = read_u16(elf_data, ELF64_E_SHNUM);
    let e_shstrndx = read_u16(elf_data, ELF64_E_SHSTRNDX);

    let shentsize = usize::from(e_shentsize);
    if shentsize < ELF64_SHENT_SIZE {
        return Err(PrescanError::MalformedSectionTable);
    }

    let shnum = usize::from(e_shnum);
    let Ok(shoff) = usize::try_from(e_shoff) else {
        return Err(PrescanError::MalformedSectionTable);
    };

    if e_shstrndx == SHN_UNDEF {
        return Ok(true);
    }
    let strndx = usize::from(e_shstrndx);
    if strndx >= shnum {
        return Err(PrescanError::MalformedSectionTable);
    }

    // Resolve the .shstrtab entry's payload range.
    let str_base = shoff
        .checked_add(
            strndx
                .checked_mul(shentsize)
                .ok_or(PrescanError::MalformedSectionTable)?,
        )
        .ok_or(PrescanError::MalformedSectionTable)?;
    let str_end_hdr = str_base
        .checked_add(shentsize)
        .ok_or(PrescanError::MalformedSectionTable)?;
    if str_end_hdr > elf_data.len() {
        return Err(PrescanError::MalformedSectionTable);
    }
    let str_sh_type = read_u32(elf_data, str_base + ELF64_SH_TYPE);
    let str_sh_offset = read_u64(elf_data, str_base + ELF64_SH_OFFSET);
    let str_sh_size = read_u64(elf_data, str_base + ELF64_SH_SIZE);

    if str_sh_type != SHT_STRTAB || str_sh_size == 0 {
        return Ok(true);
    }
    let Ok(strtab_off) = usize::try_from(str_sh_offset) else {
        return Err(PrescanError::MalformedSectionTable);
    };
    let Ok(strtab_sz) = usize::try_from(str_sh_size) else {
        return Err(PrescanError::MalformedSectionTable);
    };
    let strtab_hi = strtab_off
        .checked_add(strtab_sz)
        .ok_or(PrescanError::MalformedSectionTable)?;
    if strtab_hi > elf_data.len() {
        return Err(PrescanError::MalformedSectionTable);
    }
    let strtab = &elf_data[strtab_off..strtab_hi];

    // Walk every qualifying executable section; return false the
    // first time we find a non-empty name. If we scan every
    // qualifying section without finding one, anonymous.
    let mut saw_qualifying = false;
    for i in 0..shnum {
        let base = shoff
            .checked_add(
                i.checked_mul(shentsize)
                    .ok_or(PrescanError::MalformedSectionTable)?,
            )
            .ok_or(PrescanError::MalformedSectionTable)?;
        let end = base
            .checked_add(shentsize)
            .ok_or(PrescanError::MalformedSectionTable)?;
        if end > elf_data.len() {
            return Err(PrescanError::MalformedSectionTable);
        }

        let sh_type = read_u32(elf_data, base + ELF64_SH_TYPE);
        let sh_flags = read_u64(elf_data, base + ELF64_SH_FLAGS);
        let sh_size = read_u64(elf_data, base + ELF64_SH_SIZE);
        if !is_executable_progbits(sh_type, sh_flags, sh_size) {
            continue;
        }
        saw_qualifying = true;

        let sh_name = read_u32(elf_data, base + ELF64_SH_NAME);
        let Ok(name_off) = usize::try_from(sh_name) else {
            return Err(PrescanError::MalformedSectionTable);
        };
        if name_off >= strtab.len() {
            return Err(PrescanError::MalformedSectionTable);
        }
        if strtab[name_off] != 0 {
            return Ok(false);
        }
    }

    // No qualifying section was found -- defer to the caller.
    // Returning `false` here means the mode logic won't flag the
    // file as anonymous when no qualifying section exists; the
    // caller is already in the `sections.is_empty()` SegmentFallback
    // branch by then so this fallback is unreachable in practice.
    Ok(saw_qualifying)
}

/// Merge a list of `[lo, hi)` ranges into a deduped, overlap-coalesced
/// set sorted ascending by `lo`. Empty / inverted ranges
/// (`hi <= lo`) are dropped.
pub fn merge_ranges(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.retain(|&(lo, hi)| lo < hi);
    if ranges.is_empty() {
        return Vec::new();
    }
    ranges.sort_unstable_by_key(|&(lo, _)| lo);
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (lo, hi) in ranges {
        if let Some(last) = out.last_mut() {
            if lo <= last.1 {
                last.1 = last.1.max(hi);
                continue;
            }
        }
        out.push((lo, hi));
    }
    out
}

#[cfg(test)]
#[path = "tests/sections_tests.rs"]
mod tests;
