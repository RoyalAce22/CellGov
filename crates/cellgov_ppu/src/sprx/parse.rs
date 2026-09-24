//! Decrypted-PRX (ELF64 type 0xFFA4) byte-level parser.
//!
//! Produces a [`ParsedPrx`] that [`crate::sprx::load_prx`] consumes; no
//! guest-memory dependency lives in this layer.

use cellgov_ps3_abi::format::elf::{
    ELF64_RELA_SIZE, ELF_HEADER_SIZE, ELF_MAGIC, ET_EXEC, ET_PRX, NID_MODULE_START, NID_MODULE_STOP,
};

use crate::loader;

use super::exports::{find_system_opd, parse_export_table, parse_module_info};
use super::phdr::{scan_phdrs, v2f, RawPhdr, SegEntry};
use super::relocated_pointer::relocate_pointer_slots;

/// Parsed decrypted PRX module ready for loading.
///
/// All vaddrs (`toc`, OPD fields, segment vaddrs) are unrelocated PRX-space
/// addresses; [`crate::sprx::load_prx`] adds the chosen base.
#[derive(Debug, Clone)]
pub struct ParsedPrx {
    /// Module name from `sys_prx_module_info_t`.
    pub name: String,
    /// Stable id derived from [`Self::name`] via FNV-1a-32.
    pub module_id: crate::prx_loader::PrxModuleId,
    /// Module TOC vaddr (unrelocated).
    pub toc: u32,
    /// Text PT_LOAD segment.
    pub text: PrxSegment,
    /// Data PT_LOAD segment.
    pub data: PrxSegment,
    /// Every PT_LOAD's vaddr, in program-header order.
    ///
    /// A relocation names its target and value segment by this index.
    /// Some SDK versions emit zero-sized PT_LOAD placeholders, and a
    /// placeholder takes an index of its own.
    pub segment_vaddrs: Vec<u64>,
    /// Non-system exported libraries.
    pub exports: Vec<PrxExportLib>,
    /// RELA entries from the PT_PRX_RELOC segment.
    pub relocations: Vec<PrxRelocation>,
    /// OPD for `module_start`, if exported.
    pub module_start: Option<PrxOpd>,
    /// OPD for `module_stop`, if exported.
    pub module_stop: Option<PrxOpd>,
}

/// PT_LOAD segment bytes plus its vaddr and sizes.
///
/// `data` holds `filesz` bytes; the loader zero-extends the
/// `memsz - filesz` BSS tail.
#[derive(Debug, Clone)]
pub struct PrxSegment {
    /// This segment's position among the module's PT_LOADs, which is
    /// how a relocation names it.
    pub index: usize,
    /// Unrelocated PRX-space vaddr of the segment.
    pub vaddr: u64,
    /// On-disk byte size.
    pub filesz: u64,
    /// In-memory byte size including BSS tail.
    pub memsz: u64,
    /// Raw `filesz` bytes from the file.
    pub data: Vec<u8>,
}

/// One exported library within a PRX module.
#[derive(Debug, Clone)]
pub struct PrxExportLib {
    /// Library name string.
    pub name: String,
    /// Library attribute flags.
    pub attrs: u16,
    /// Exported function entries.
    pub functions: Vec<PrxExport>,
    /// Exported variable entries.
    pub variables: Vec<PrxExport>,
}

/// One exported symbol; `vaddr` is unrelocated PRX-space.
#[derive(Debug, Clone, Copy)]
pub struct PrxExport {
    /// Symbol NID.
    pub nid: u32,
    /// Unrelocated PRX-space vaddr of the symbol's stub.
    pub vaddr: u32,
}

/// Official Procedure Descriptor: function entry point and TOC pair.
///
/// All three fields are unrelocated absolute PRX vaddrs, not segment-relative
/// offsets -- adding `text.vaddr` would double-count for non-zero-based text.
#[derive(Debug, Clone, Copy)]
pub struct PrxOpd {
    /// Vaddr of the OPD itself.
    pub opd_vaddr: u32,
    /// Function entry-point vaddr.
    pub code: u32,
    /// TOC vaddr paired with this entry point.
    pub toc: u32,
}

/// One ELF64 RELA relocation entry.
///
/// `sym` packs two PT_LOAD indices:
///
/// - `sym & 0xFF` names the segment to patch.
/// - `(sym >> 8) & 0xFF` names the segment whose vaddr the `addend` is
///   relative to, or [`PRX_RELOC_NO_VALUE_SEGMENT`] for a whole address.
///
/// [`PRX_RELOC_NO_VALUE_SEGMENT`]: cellgov_ps3_abi::format::elf::PRX_RELOC_NO_VALUE_SEGMENT
#[derive(Debug, Clone, Copy)]
pub struct PrxRelocation {
    /// Offset within the target segment to patch.
    pub offset: u64,
    /// PPC64 relocation type code.
    pub rtype: u32,
    /// Packed target/value segment indices (low byte / next byte).
    pub sym: u32,
    /// Signed addend added to the value-segment vaddr.
    pub addend: i64,
}

/// Failure mode while parsing a decrypted PRX.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrxParseError {
    /// Input is shorter than the ELF header.
    #[error("PRX too small for ELF header")]
    TooSmall,
    /// First four bytes are not the ELF magic.
    #[error("PRX bad ELF magic")]
    BadMagic,
    /// ELF class/encoding is not ELF64 big-endian.
    #[error("PRX is not ELF64 big-endian")]
    NotElf64Be,
    /// ELF e_type was not 0xFFA4 (PS3 PRX); carries the observed type.
    #[error("PRX e_type 0x{0:04x} is not 0xFFA4")]
    NotPrx(u16),
    /// A PS3 module carries two content-bearing PT_LOAD segments;
    /// this one carries the reported count.
    #[error("PRX has {0} PT_LOAD segment(s) with content, expected text and data")]
    MissingSegments(usize),
    /// A computed file offset or size escaped the input buffer.
    #[error("PRX offset or size escaped buffer")]
    OutOfBounds,
    /// `sys_prx_module_info_t` was missing or unreadable.
    #[error("PRX sys_prx_module_info_t missing")]
    NoModuleInfo,
}

/// The module identity a PPU object carries, or `None` for a title
/// executable.
///
/// `Ok(None)` is the title-executable case: `e_type` is `ET_EXEC`, so
/// no `sys_prx_module_info_t` exists and none is expected. A PPU
/// object on this platform carries one of exactly two ELF types, both
/// in [`cellgov_ps3_abi::format::elf`]. `ET_EXEC` names a title
/// executable. The PS3 relocatable-module type names every firmware
/// module under `dev_flash/sys/external`.
///
/// # Errors
///
/// Any [`parse_prx`] refusal of a file whose type is not `ET_EXEC`.
/// Every other `e_type` is a structural anomaly, so it answers
/// [`PrxParseError::NotPrx`] rather than reading as an executable.
pub fn module_identity(data: &[u8]) -> Result<Option<ParsedPrx>, PrxParseError> {
    match parse_prx(data) {
        Ok(parsed) => Ok(Some(parsed)),
        Err(PrxParseError::NotPrx(e_type)) if e_type == ET_EXEC => Ok(None),
        Err(error) => Err(error),
    }
}

/// Parse a decrypted PRX (ELF64 type 0xFFA4) into its components.
///
/// Input must already be decrypted, not a raw SCE-encrypted SELF. Unwrap one
/// with `cellgov self decrypt` from a build carrying the `decrypt` feature, or
/// through `cellgov dev prx-imports`, which detects the SCE wrapper and
/// decrypts before parsing.
pub fn parse_prx(data: &[u8]) -> Result<ParsedPrx, PrxParseError> {
    if data.len() < ELF_HEADER_SIZE {
        return Err(PrxParseError::TooSmall);
    }
    if data[0..4] != ELF_MAGIC {
        return Err(PrxParseError::BadMagic);
    }
    if data[4] != 2 || data[5] != 2 {
        return Err(PrxParseError::NotElf64Be);
    }
    let e_type = loader::read_u16(data, 16);
    if e_type != ET_PRX {
        return Err(PrxParseError::NotPrx(e_type));
    }

    let (loads, reloc_phdr) = scan_phdrs(data)?;

    // A module carries text and data. Some SDK versions surround them
    // with zero-sized PT_LOAD placeholders, which take a relocation
    // index and hold nothing to place.
    let content: Vec<usize> = loads
        .iter()
        .enumerate()
        .filter(|(_, l)| l.p_memsz > 0)
        .map(|(i, _)| i)
        .collect();
    let [text_idx, data_idx] = content[..] else {
        return Err(PrxParseError::MissingSegments(content.len()));
    };

    let seg_map: Vec<SegEntry> = loads
        .iter()
        .filter(|l| l.p_filesz > 0)
        .map(|l| SegEntry {
            vaddr: l.p_vaddr as usize,
            file_offset: l.p_offset,
            size: l.p_filesz as usize,
        })
        .collect();

    let text = extract_segment(data, &loads[text_idx], text_idx)?;
    let data_seg = extract_segment(data, &loads[data_idx], data_idx)?;

    let relocations = match reloc_phdr {
        Some(rp) => parse_relocations(data, &rp)?,
        None => Vec::new(),
    };
    let image = relocate_pointer_slots(data, &loads, &relocations)
        .map_err(|_| PrxParseError::OutOfBounds)?;

    // The text segment's p_paddr doubles as the file offset of
    // module_info.
    let mi_file_off = loads[text_idx].p_paddr as usize;
    let (name, toc, exports_range, _imports_range) = parse_module_info(&image, mi_file_off)?;

    let exports = parse_export_table(&image, &seg_map, exports_range)?;

    let module_start = find_system_opd(&image, &seg_map, &exports_range, NID_MODULE_START)?;
    let module_stop = find_system_opd(&image, &seg_map, &exports_range, NID_MODULE_STOP)?;

    let module_id = crate::prx_loader::graph::module_id_from_name(&name);
    Ok(ParsedPrx {
        name,
        module_id,
        toc,
        text,
        data: data_seg,
        segment_vaddrs: loads.iter().map(|l| l.p_vaddr).collect(),
        exports,
        relocations,
        module_start,
        module_stop,
    })
}

pub(super) fn extract_segment(
    data: &[u8],
    phdr: &RawPhdr,
    index: usize,
) -> Result<PrxSegment, PrxParseError> {
    // ELF requires p_memsz >= p_filesz. The loader sizes its region
    // check against memsz, so filesz > memsz would write past the
    // validated range.
    if phdr.p_filesz > phdr.p_memsz {
        return Err(PrxParseError::OutOfBounds);
    }
    let end = phdr
        .p_offset
        .checked_add(phdr.p_filesz as usize)
        .ok_or(PrxParseError::OutOfBounds)?;
    if end > data.len() {
        return Err(PrxParseError::OutOfBounds);
    }
    Ok(PrxSegment {
        index,
        vaddr: phdr.p_vaddr,
        filesz: phdr.p_filesz,
        memsz: phdr.p_memsz,
        data: data[phdr.p_offset..end].to_vec(),
    })
}

/// Parse RELA entries from the 0x700000A4 relocation segment.
pub(super) fn parse_relocations(
    data: &[u8],
    phdr: &RawPhdr,
) -> Result<Vec<PrxRelocation>, PrxParseError> {
    let start = phdr.p_offset;
    let size = phdr.p_filesz as usize;
    // Both fields are unvalidated header words from an arbitrary file:
    // `relocated_pointer_image` reaches this before any PRX check.
    let end = start.checked_add(size).ok_or(PrxParseError::OutOfBounds)?;
    if end > data.len() {
        return Err(PrxParseError::OutOfBounds);
    }

    let count = size / ELF64_RELA_SIZE;
    let mut relocs = Vec::with_capacity(count);

    for i in 0..count {
        let off = start + i * ELF64_RELA_SIZE;
        let r_offset = loader::read_u64(data, off);
        let r_info = loader::read_u64(data, off + 8);
        let r_addend = loader::read_u64(data, off + 16) as i64;
        let r_sym = (r_info >> 32) as u32;
        let r_type = (r_info & 0xFFFF_FFFF) as u32;

        relocs.push(PrxRelocation {
            offset: r_offset,
            rtype: r_type,
            sym: r_sym,
            addend: r_addend,
        });
    }

    Ok(relocs)
}

pub(super) fn read_cstring(data: &[u8], seg_map: &[SegEntry], vaddr: usize) -> String {
    // 256 is comfortably above any real PRX library / module name
    // and consistent with ELF SHT_STRTAB conventions. A corrupt
    // name pointer that aims at unterminated bytes would otherwise
    // return hundreds of KB of segment content as a "name".
    const MAX_CSTRING_LEN: usize = 256;
    // Failed lookups embed the vaddr so corrupt name pointers stay
    // distinguishable from legitimately-empty strings downstream.
    let foff = match v2f(seg_map, vaddr) {
        Some(o) => o,
        None => return format!("<unmapped:0x{vaddr:x}>"),
    };
    if foff >= data.len() {
        return format!("<oob:0x{vaddr:x}>");
    }
    let scan_end = (foff + MAX_CSTRING_LEN).min(data.len());
    let end = data[foff..scan_end]
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(scan_end - foff);
    String::from_utf8_lossy(&data[foff..foff + end]).into_owned()
}

#[cfg(test)]
#[path = "tests/parse_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/module_identity_tests.rs"]
mod module_identity_tests;

#[cfg(test)]
#[path = "tests/placeholder_segment_tests.rs"]
mod placeholder_segment_tests;
