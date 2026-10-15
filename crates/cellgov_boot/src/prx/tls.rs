//! TLS pre-init from the ELF's PT_TLS segment and the synthetic
//! kernel-context OPD that liblv2's entry expects in r11 / r12.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

use crate::BootSink;

/// Offset (from the area base) at which the PT_TLS template starts; PS3
/// kernel convention leaves `0x30` bytes of scratch for the per-thread
/// TLS header.
///
/// [`pre_init_tls`] zeroes those bytes with the template, so the area
/// holds one known image. `seed_tls_thread_id` then writes the
/// thread-id word at the area base; the rest stays zero.
const TLS_TEMPLATE_OFFSET: u64 = 0x30;

/// The main thread's TLS area in one process: 64 KB on a 64 KB page,
/// with the synthetic kernel-context OPD in its last 16 bytes.
///
/// CellGov puts the area on the first 64 KB page at or above the end of
/// everything resident, so it never overlaps the image or the firmware
/// set. The console uses a
/// different address. Its captures `micro:lv2_process_layout` and
/// `micro:lv2_process_layout_wide` read the same r13 (`0x30007060`) at
/// main for both image extents. There the TLS sits in the user-memory
/// block at `0x30000000`, which does not move with the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsArea {
    base: u64,
}

impl TlsArea {
    /// Size of the area in bytes; every area base is a multiple of it.
    pub const SIZE: u64 = 0x1_0000;

    /// Offset of the kernel-context OPD slot: the last 16 bytes.
    const OPD_OFFSET: u64 = Self::SIZE - 16;

    /// The area on the first 64 KB page at or above `resident_end`.
    ///
    /// `resident_end` is the exclusive end of everything already
    /// resident. Returns `None` when the area would pass the end of the
    /// 64-bit address range.
    pub fn above(resident_end: u64) -> Option<Self> {
        let base = resident_end.checked_next_multiple_of(Self::SIZE)?;
        base.checked_add(Self::SIZE)?;
        Some(Self { base })
    }

    /// First address of the area: the per-thread TLS header.
    pub fn base(self) -> u64 {
        self.base
    }

    /// First address past the area.
    pub fn end(self) -> u64 {
        self.base + Self::SIZE
    }

    /// Guest address of the kernel-context OPD slot.
    pub fn kernel_context_opd(self) -> u64 {
        self.base + Self::OPD_OFFSET
    }

    /// The r13 a thread starts with: the template start plus `0x7000`,
    /// the PPC64 TLS bias.
    pub fn thread_pointer(self) -> u64 {
        self.base + TLS_TEMPLATE_OFFSET + 0x7000
    }
}

/// Why TLS or the kernel-context OPD could not be initialized.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// The PT_TLS template would reach into the kernel-context OPD slot.
    #[error(
        "tls: PT_TLS memsz=0x{memsz:x} extends past offset 0x{opd_offset:x} \
         (kernel-context OPD slot); shrink the template or relocate the OPD"
    )]
    TemplateOverlapsOpd {
        /// The PT_TLS memory size.
        memsz: usize,
        /// Offset of the OPD slot inside the area.
        opd_offset: usize,
    },
    /// The PT_TLS header claims more file bytes than memory bytes, which
    /// the ELF format forbids. `p_memsz` sizes the per-thread image;
    /// `p_filesz` sizes only its initialized head.
    #[error("tls: PT_TLS filesz 0x{filesz:x} exceeds memsz 0x{memsz:x}")]
    TemplateLongerThanImage {
        /// Declared file-image size.
        filesz: usize,
        /// Declared memory-image size.
        memsz: usize,
    },
    /// The template's source range overflows the host's address
    /// arithmetic.
    #[error("tls: PT_TLS vaddr=0x{vaddr:x} + filesz=0x{filesz:x} overflows usize")]
    SourceOverflow {
        /// The PT_TLS load address.
        vaddr: usize,
        /// Its file size.
        filesz: usize,
    },
    /// The template's source range is outside guest memory.
    #[error("tls: PT_TLS src 0x{vaddr:x}+0x{filesz:x} exceeds guest memory ({mem_len} bytes)")]
    SourceOutOfRange {
        /// The PT_TLS load address.
        vaddr: usize,
        /// Its file size.
        filesz: usize,
        /// Bytes the address space holds.
        mem_len: usize,
    },
    /// The template's destination range overflows the host's address
    /// arithmetic.
    #[error("tls: TLS dst 0x{dst:x} + memsz=0x{memsz:x} overflows usize")]
    DestOverflow {
        /// Where the template would be written.
        dst: usize,
        /// Its memory size.
        memsz: usize,
    },
    /// The template's destination range is outside guest memory.
    #[error("tls: TLS dst 0x{dst:x}+0x{memsz:x} exceeds guest memory ({mem_len} bytes)")]
    DestOutOfRange {
        /// Where the template would be written.
        dst: usize,
        /// Its memory size.
        memsz: usize,
        /// Bytes the address space holds.
        mem_len: usize,
    },
    /// The destination is not an addressable range.
    #[error("tls: invalid byte range at 0x{dst:016x}")]
    DestBadRange {
        /// Base of the area that receives the image.
        dst: usize,
    },
    /// The pre-init commit was refused, so TLS holds its pre-load bytes.
    #[error("tls: pre-init commit at 0x{dst:016x} FAILED ({source}); TLS not initialized")]
    Commit {
        /// Base of the area that receives the image.
        dst: usize,
        /// The commit pipeline's own account of the refusal.
        source: cellgov_mem::MemError,
    },
    /// The kernel-context OPD slot is past the 32-bit address range an
    /// OPD code word can name.
    #[error(
        "module_start: kernel-context OPD slot 0x{addr:016x} is past the 32-bit address range"
    )]
    KernelContextOpdRange {
        /// Guest address of the OPD slot.
        addr: u64,
    },
    /// The kernel-context OPD commit was refused.
    #[error(
        "module_start: kernel-context OPD install at 0x{addr:016x} FAILED ({source}); \
         liblv2 module_start would fault on the entry r11/r12 path"
    )]
    KernelContextOpd {
        /// Guest address of the OPD slot.
        addr: u64,
        /// The commit pipeline's own account of the refusal.
        source: cellgov_mem::MemError,
    },
}

/// Pre-initialize TLS from the ELF's PT_TLS segment.
///
/// Stages the `0x30` header gap, the template bytes and any BSS tail
/// into a single buffer, then commits with one `apply_commit` so the
/// guest never observes a partially initialized TLS image. PS3 LV2
/// performs this during process creation before any module_start runs.
///
/// The caller must place the image's PT_LOAD segments first. `elf_data`
/// only locates the PT_TLS header; this function reads the template
/// bytes back out of `mem` at `p_vaddr`, where the segment loader put
/// them. Without that order the template carries whatever the address
/// holds, and nothing in the signature expresses the requirement.
///
/// # Errors
///
/// The PT_TLS header is malformed, the template does not fit the
/// area or guest memory, or the one commit was refused; see
/// [`TlsError`].
pub fn pre_init_tls(
    elf_data: &[u8],
    mem: &mut GuestMemory,
    area: TlsArea,
    sink: &dyn BootSink,
) -> Result<(), TlsError> {
    let tls_base = area.base();
    let tls = match cellgov_ppu::loader::find_tls_segment(elf_data) {
        Some(t) => t,
        None => return Ok(()),
    };

    let p_vaddr = tls.vaddr as usize;
    let p_filesz = tls.filesz as usize;
    let p_memsz = tls.memsz as usize;
    // Ordered ahead of the zero-memsz skip, which reads
    // `filesz > 0, memsz == 0` as an absent template. The pair is
    // malformed, and `cellgov_ppu::loader` refuses it on a PT_LOAD too.
    // The check also keeps the copy below in range: the staging buffer
    // is memsz bytes, and the template head fills its first filesz.
    if p_filesz > p_memsz {
        return Err(TlsError::TemplateLongerThanImage {
            filesz: p_filesz,
            memsz: p_memsz,
        });
    }
    if p_memsz == 0 {
        return Ok(());
    }

    // Reject a PT_TLS that would extend into the kernel-context OPD
    // slot at the top of the area: the OPD commit happens later
    // and would clobber the template tail otherwise.
    let opd_offset = TlsArea::OPD_OFFSET as usize;
    let template_end_offset =
        (TLS_TEMPLATE_OFFSET as usize)
            .checked_add(p_memsz)
            .ok_or(TlsError::DestOverflow {
                dst: tls_base as usize + TLS_TEMPLATE_OFFSET as usize,
                memsz: p_memsz,
            })?;
    if template_end_offset > opd_offset {
        return Err(TlsError::TemplateOverlapsOpd {
            memsz: p_memsz,
            opd_offset,
        });
    }

    let m_len = mem.as_bytes().len();
    let tls_data_start = tls_base as usize + TLS_TEMPLATE_OFFSET as usize;

    let src_end = p_vaddr
        .checked_add(p_filesz)
        .ok_or(TlsError::SourceOverflow {
            vaddr: p_vaddr,
            filesz: p_filesz,
        })?;
    if src_end > m_len {
        return Err(TlsError::SourceOutOfRange {
            vaddr: p_vaddr,
            filesz: p_filesz,
            mem_len: m_len,
        });
    }
    let dst_end = tls_data_start
        .checked_add(p_memsz)
        .ok_or(TlsError::DestOverflow {
            dst: tls_data_start,
            memsz: p_memsz,
        })?;
    if dst_end > m_len {
        return Err(TlsError::DestOutOfRange {
            dst: tls_data_start,
            memsz: p_memsz,
            mem_len: m_len,
        });
    }

    // One image over the header gap and the template, so the header
    // reads zero rather than whatever the area held.
    let header_len = TLS_TEMPLATE_OFFSET as usize;
    let mut image = vec![0u8; template_end_offset];
    if p_filesz > 0 {
        let m = mem.as_bytes();
        image[header_len..header_len + p_filesz].copy_from_slice(&m[p_vaddr..src_end]);
    }
    let reservation = tls_base as usize;
    let range = ByteRange::new(GuestAddr::new(tls_base), image.len() as u64)
        .ok_or(TlsError::DestBadRange { dst: reservation })?;
    mem.apply_commit(range, &image)
        .map_err(|source| TlsError::Commit {
            dst: reservation,
            source,
        })?;

    sink.note(&format!(
        "tls: pre-initialized from PT_TLS at 0x{:x} (filesz=0x{:x}, memsz=0x{:x}) -> 0x{:x}, \
         header 0x{:x}..0x{:x} zeroed",
        p_vaddr, p_filesz, p_memsz, tls_data_start, tls_base, tls_data_start
    ));
    Ok(())
}

/// Write a `{code, toc}` OPD whose body is a single `blr` into the
/// last 16 bytes of `area` and return its address. liblv2's entry
/// expects kernel-side function OPDs in r11 / r12; the synthetic OPD
/// lets those calls return cleanly.
///
/// # Errors
///
/// - [`TlsError::KernelContextOpdRange`]: the slot is past the 32-bit
///   address range that an OPD word can name.
/// - [`TlsError::KernelContextOpd`]: the commit pipeline refuses the
///   write.
pub fn install_kernel_context_opd(mem: &mut GuestMemory, area: TlsArea) -> Result<u64, TlsError> {
    let opd_addr = area.kernel_context_opd();
    let blr_addr = u32::try_from(opd_addr + 8)
        .map_err(|_| TlsError::KernelContextOpdRange { addr: opd_addr })?;
    let mut bytes = [0u8; 16];
    bytes[0..4].copy_from_slice(&blr_addr.to_be_bytes());
    bytes[4..8].copy_from_slice(&0u32.to_be_bytes());
    bytes[8..12].copy_from_slice(&0x4e80_0020u32.to_be_bytes());
    let range = ByteRange::new(GuestAddr::new(opd_addr), 16)
        .ok_or(TlsError::KernelContextOpdRange { addr: opd_addr })?;
    mem.apply_commit(range, &bytes)
        .map_err(|source| TlsError::KernelContextOpd {
            addr: opd_addr,
            source,
        })?;
    Ok(opd_addr)
}

#[cfg(test)]
#[path = "tests/tls_tests.rs"]
mod tests;
