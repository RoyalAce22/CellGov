//! A PRX image with its ADDR32 pointer slots resolved, for a reader that does not parse the module.

use std::borrow::Cow;

use cellgov_ps3_abi::format::elf::{
    ELF_HEADER_SIZE, ELF_MAGIC, ELF_PHENTSIZE, ET_PRX, PRX_RELOC_NO_VALUE_SEGMENT, R_PPC64_ADDR32,
};

use crate::loader;

use super::parse::{parse_relocations, PrxRelocation};
use super::phdr::{scan_phdrs, RawPhdr};

/// `data` with every `R_PPC64_ADDR32` slot resolved, for a caller that
/// reads pointers out of a PRX without parsing one.
///
/// Copies the whole file to rewrite the slots. Borrows `data` instead
/// when the file:
///
/// - declares no PT_PRX_RELOC segment,
/// - declares fewer than two PT_LOADs, as a title executable does,
/// - carries a program-header table that does not scan.
///
/// See `relocate_pointer_slots` for what the raw slot word holds.
///
/// # Errors
///
/// Returns [`RelocatedPointerError`] when relocation metadata escapes the
/// file or an ADDR32 pointer-slot rewrite would lose data.
pub(crate) fn relocated_pointer_image(data: &[u8]) -> Result<Cow<'_, [u8]>, RelocatedPointerError> {
    if data.len() < ELF_HEADER_SIZE {
        return Err(RelocatedPointerError::OutOfBounds);
    }
    if data[0..4] != ELF_MAGIC || loader::read_u16(data, 16) != ET_PRX {
        return Ok(Cow::Borrowed(data));
    }
    let phentsize = loader::read_u16(data, 54) as usize;
    if phentsize < ELF_PHENTSIZE {
        return Err(RelocatedPointerError::BadPhentsize { phentsize });
    }
    let Ok((loads, reloc_phdr)) = scan_phdrs(data) else {
        return Ok(Cow::Borrowed(data));
    };
    let Some(reloc_phdr) = reloc_phdr else {
        return Ok(Cow::Borrowed(data));
    };
    if loads.len() < 2 {
        return Ok(Cow::Borrowed(data));
    }
    let relocs =
        parse_relocations(data, &reloc_phdr).map_err(|_| RelocatedPointerError::OutOfBounds)?;
    Ok(Cow::Owned(relocate_pointer_slots(data, &loads, &relocs)?))
}

/// Failure while preparing the relocated metadata view used by import parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RelocatedPointerError {
    /// A declared relocation segment escapes the file.
    #[error("declared relocation metadata escapes the file")]
    OutOfBounds,
    /// An ELF64 program-header slot is smaller than its fixed layout.
    #[error("ELF64 program-header slot is {phentsize} bytes, below the required 56 bytes")]
    BadPhentsize {
        /// Declared slot size.
        phentsize: usize,
    },
    /// An ADDR32 patch offset is not naturally aligned.
    #[error("ADDR32 patch offset 0x{offset:x} is not 4-byte aligned")]
    RelocPatchMisaligned {
        /// Offset within the target segment.
        offset: u64,
    },
    /// An ADDR32 value needs bits above bit 31.
    #[error("ADDR32 relocation value 0x{value:x} does not fit in 32 bits")]
    RelocOverflow {
        /// Computed relocation value.
        value: u64,
    },
}

struct PointerRelocation {
    slot: std::ops::Range<usize>,
    value: u32,
}

fn pointer_relocation(
    loads: &[RawPhdr],
    r: &PrxRelocation,
) -> Result<Option<PointerRelocation>, RelocatedPointerError> {
    if r.rtype != R_PPC64_ADDR32 {
        return Ok(None);
    }
    // The loader rejects such an index. Skip the slot so the loader
    // reports the bad index once.
    let Some(target) = loads.get((r.sym & 0xFF) as usize) else {
        return Ok(None);
    };
    let Some(value_base) = value_segment_base(loads, r.sym) else {
        return Ok(None);
    };
    if r.offset & 3 != 0 {
        return Err(RelocatedPointerError::RelocPatchMisaligned { offset: r.offset });
    }
    let offset_end = r
        .offset
        .checked_add(4)
        .ok_or(RelocatedPointerError::OutOfBounds)?;
    // A slot past filesz is BSS: it has no file bytes to rewrite.
    if offset_end > target.p_filesz {
        return Ok(None);
    }

    let slot_start = (target.p_offset as u64)
        .checked_add(r.offset)
        .and_then(|slot| usize::try_from(slot).ok())
        .ok_or(RelocatedPointerError::OutOfBounds)?;
    let slot_end = slot_start
        .checked_add(4)
        .ok_or(RelocatedPointerError::OutOfBounds)?;
    let value = value_base.wrapping_add(r.addend as u64);
    if value >> 32 != 0 {
        return Err(RelocatedPointerError::RelocOverflow { value });
    }

    Ok(Some(PointerRelocation {
        slot: slot_start..slot_end,
        value: value as u32,
    }))
}

/// Copy of the file image with every `R_PPC64_ADDR32` slot holding the
/// PRX-space address it resolves to.
///
/// PRX metadata pointers -- module-info, export tables, OPD words,
/// import tables -- are relocation targets. Some SDK versions store the
/// bare addend in the slot and let the relocation supply the value
/// segment's vaddr; others store the sum. A raw read of the first kind
/// lands short by that vaddr, in the wrong segment.
///
/// Only the metadata reads take this image; [`PrxSegment::data`](crate::sprx::PrxSegment::data) keeps
/// the file's own bytes for [`crate::sprx::load_prx`] to relocate
/// against the real base.
pub(super) fn relocate_pointer_slots(
    data: &[u8],
    loads: &[RawPhdr],
    relocs: &[PrxRelocation],
) -> Result<Vec<u8>, RelocatedPointerError> {
    let mut image = data.to_vec();
    for r in relocs {
        let Some(pointer) = pointer_relocation(loads, r)? else {
            continue;
        };
        let bytes = image
            .get_mut(pointer.slot)
            .ok_or(RelocatedPointerError::OutOfBounds)?;
        bytes.copy_from_slice(&pointer.value.to_be_bytes());
    }
    Ok(image)
}

/// PRX-space base for a relocation's addend; zero when `sym` names no
/// value segment.
///
/// Returns `None` when `sym` names a PT_LOAD the module does not
/// declare, or one with no memory. The loader never allocates a
/// zero-sized placeholder, so it has no address.
fn value_segment_base(loads: &[RawPhdr], sym: u32) -> Option<u64> {
    match (sym >> 8) & 0xFF {
        PRX_RELOC_NO_VALUE_SEGMENT => Some(0),
        idx => loads
            .get(idx as usize)
            .filter(|l| l.p_memsz > 0)
            .map(|l| l.p_vaddr),
    }
}

#[cfg(test)]
#[path = "tests/relocated_pointer_tests.rs"]
mod relocated_pointer_tests;
