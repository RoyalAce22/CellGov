//! The kernel ELF's segments, placed at their link addresses.

use cellgov_mem::{be::read_u64, ByteRange, GuestAddr, GuestMemory, MemError, PageSize, Region};
use cellgov_ppu::loader::{checked_pt_loads, LoadError, LoadSegment};
use cellgov_ps3_abi::format::core_os::LV2_KERNEL_LINK_BASE;
use cellgov_ps3_abi::format::elf::ELF64_E_ENTRY;

/// The largest span, in bytes, that a kernel's segments may cover.
///
/// CellGov sets this bound so that no header can size the region past
/// it. The largest stored kernel, 1.94's, spans 0x4b55c8 bytes.
pub(crate) const KERNEL_SPAN_LIMIT: u64 = 64 << 20;

/// The kernel region's page size, and the alignment of its two ends.
const KERNEL_PAGE: u64 = 0x1_0000;

/// A kernel mapped at its link address.
#[derive(Debug)]
pub struct KernelImage {
    /// One region, `lv2`, that covers every segment. It holds the
    /// kernel's bytes and marks each executable segment as code.
    pub mem: GuestMemory,
    /// The ELF's entry: the first instruction itself, never a function
    /// descriptor.
    pub entry: u64,
}

/// Why a kernel ELF does not map.
#[derive(Debug, thiserror::Error)]
pub enum KernelMapError {
    /// The header or a program header does not parse.
    #[error("kernel ELF: {source}")]
    Elf {
        /// The loader's own account of the refusal.
        source: LoadError,
    },
    /// No PT_LOAD has a memory size.
    #[error("kernel ELF has no loadable segment")]
    NoSegment,
    /// A segment lies below the kernel's link base, so the file is no
    /// kernel.
    #[error("kernel segment {segment_index} at 0x{vaddr:016x} lies below the link base 0x{LV2_KERNEL_LINK_BASE:016x}")]
    BelowLinkBase {
        /// The program header's index.
        segment_index: usize,
        /// Where it asks to load.
        vaddr: u64,
    },
    /// The segments span more than the kernel span limit of 64 MiB.
    #[error(
        "kernel segments span 0x{span:08x} bytes, past the limit of 0x{KERNEL_SPAN_LIMIT:08x}"
    )]
    SpanTooLarge {
        /// From the lowest segment start, rounded down to 64 KiB, to the highest segment end.
        span: u64,
    },
    /// The entry is not in the file bytes of an executable segment.
    #[error("kernel entry 0x{entry:016x} lies in no executable segment")]
    EntryOutsideCode {
        /// The ELF's entry.
        entry: u64,
    },
    /// The address space refused the region or a segment's commit.
    #[error("kernel address space: {source}")]
    Layout {
        /// The address space's own account of the refusal.
        source: MemError,
    },
}

/// Maps the kernel ELF `elf` at its link address.
///
/// # Errors
///
/// Returns a [`KernelMapError`]. Every check but `Layout` runs before
/// the region exists.
pub(crate) fn map_kernel(elf: &[u8]) -> Result<KernelImage, KernelMapError> {
    let segments: Vec<LoadSegment> = checked_pt_loads(elf)
        .map_err(|source| KernelMapError::Elf { source })?
        .into_iter()
        .filter(|s| s.memsz != 0)
        .collect();
    if let Some(low) = segments.iter().find(|s| s.vaddr < LV2_KERNEL_LINK_BASE) {
        return Err(KernelMapError::BelowLinkBase {
            segment_index: low.index,
            vaddr: low.vaddr,
        });
    }
    // checked_pt_loads refused a segment whose end overflows.
    let start = segments
        .iter()
        .map(|s| s.vaddr)
        .min()
        .ok_or(KernelMapError::NoSegment)?
        & !(KERNEL_PAGE - 1);
    let end = segments
        .iter()
        .map(|s| s.vaddr + s.memsz)
        .max()
        .unwrap_or(start);
    let span = end - start;
    if span > KERNEL_SPAN_LIMIT {
        return Err(KernelMapError::SpanTooLarge { span });
    }
    let entry = read_u64(elf, ELF64_E_ENTRY);
    let entry_in_code = segments.iter().any(|s| {
        s.executable
            && entry >= s.vaddr
            && entry
                .checked_add(4)
                .is_some_and(|after| after <= s.vaddr + s.filesz)
    });
    if !entry_in_code {
        return Err(KernelMapError::EntryOutsideCode { entry });
    }

    let size = span.next_multiple_of(KERNEL_PAGE) as usize;
    let mut mem =
        GuestMemory::from_regions(vec![Region::new(start, size, "lv2", PageSize::Page64K)])
            .map_err(|source| KernelMapError::Layout { source })?;
    for s in &segments {
        // A fresh region reads zero, so a segment's BSS needs no write.
        let file_bytes = ByteRange::new(GuestAddr::new(s.vaddr), s.filesz).filter(|_| s.filesz > 0);
        if let Some(range) = file_bytes {
            let from = s.file_offset as usize;
            mem.apply_commit(range, &elf[from..from + s.filesz as usize])
                .map_err(|source| KernelMapError::Layout { source })?;
        }
        if s.executable {
            mem.declare_code(s.vaddr, s.memsz);
        }
    }
    Ok(KernelImage { mem, entry })
}
