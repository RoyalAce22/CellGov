//! TLS pre-init from the ELF's PT_TLS segment and the synthetic
//! kernel-context OPD that liblv2's entry expects in r11 / r12.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

use crate::prepare::HLE_HEAP_BASE;
use crate::BootSink;

/// Must match the firmware sysPrxForUser `sys_initialize_tls`
/// allocation seen by liblv2 at module_start time.
pub const TLS_BASE: u64 = 0x10400000;

/// Guest address of the synthetic kernel-context OPD installed by
/// [`install_kernel_context_opd`]. Sits in the last 16 bytes of the
/// 64 KB TLS reservation, immediately below `HLE_HEAP_BASE`.
const KERNEL_CTX_OPD_ADDR: u64 = 0x1040_FFF0;

const _: () = assert!(KERNEL_CTX_OPD_ADDR > TLS_BASE);
const _: () = assert!(KERNEL_CTX_OPD_ADDR + 16 == HLE_HEAP_BASE as u64);

/// Offset (from `TLS_BASE`) at which the PT_TLS template starts; PS3
/// kernel convention leaves `0x30` bytes of scratch for the per-thread
/// TLS header.
///
/// [`pre_init_tls`] zeroes those bytes with the template, so the
/// reservation holds one known image. `seed_tls_thread_id` then writes
/// the thread-id word at `TLS_BASE`; the rest stays zero.
const TLS_TEMPLATE_OFFSET: u64 = 0x30;

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
        /// Offset of the OPD slot inside the reservation.
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
        /// Base of the reservation the image would be committed to.
        dst: usize,
    },
    /// The pre-init commit was refused, so TLS holds its pre-load bytes.
    #[error("tls: pre-init commit at 0x{dst:016x} FAILED ({source}); TLS not initialized")]
    Commit {
        /// Base of the reservation the image would be committed to.
        dst: usize,
        /// The commit pipeline's own account of the refusal.
        source: cellgov_mem::MemError,
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
/// reservation or guest memory, or the one commit was refused; see
/// [`TlsError`].
pub fn pre_init_tls(
    elf_data: &[u8],
    mem: &mut GuestMemory,
    sink: &dyn BootSink,
) -> Result<(), TlsError> {
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
    // slot at the top of the reservation: the OPD commit happens later
    // and would clobber the template tail otherwise.
    let opd_offset = (KERNEL_CTX_OPD_ADDR - TLS_BASE) as usize;
    let template_end_offset =
        (TLS_TEMPLATE_OFFSET as usize)
            .checked_add(p_memsz)
            .ok_or(TlsError::DestOverflow {
                dst: TLS_BASE as usize + TLS_TEMPLATE_OFFSET as usize,
                memsz: p_memsz,
            })?;
    if template_end_offset > opd_offset {
        return Err(TlsError::TemplateOverlapsOpd {
            memsz: p_memsz,
            opd_offset,
        });
    }

    let m_len = mem.as_bytes().len();
    let tls_data_start = TLS_BASE as usize + TLS_TEMPLATE_OFFSET as usize;

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
    // reads zero rather than whatever the reservation held.
    let header_len = TLS_TEMPLATE_OFFSET as usize;
    let mut image = vec![0u8; template_end_offset];
    if p_filesz > 0 {
        let m = mem.as_bytes();
        image[header_len..header_len + p_filesz].copy_from_slice(&m[p_vaddr..src_end]);
    }
    let reservation = TLS_BASE as usize;
    let range = ByteRange::new(GuestAddr::new(TLS_BASE), image.len() as u64)
        .ok_or(TlsError::DestBadRange { dst: reservation })?;
    mem.apply_commit(range, &image)
        .map_err(|source| TlsError::Commit {
            dst: reservation,
            source,
        })?;

    sink.note(&format!(
        "tls: pre-initialized from PT_TLS at 0x{:x} (filesz=0x{:x}, memsz=0x{:x}) -> 0x{:x}, \
         header 0x{:x}..0x{:x} zeroed",
        p_vaddr, p_filesz, p_memsz, tls_data_start, TLS_BASE, tls_data_start
    ));
    Ok(())
}

/// Write a `{code, toc}` OPD whose body is a single `blr` and return
/// its address. liblv2's entry expects kernel-side function OPDs in
/// r11 / r12; the synthetic OPD lets those calls return cleanly.
///
/// # Errors
///
/// [`TlsError::KernelContextOpd`] when the commit is refused.
pub fn install_kernel_context_opd(mem: &mut GuestMemory) -> Result<u64, TlsError> {
    let opd_addr = KERNEL_CTX_OPD_ADDR;
    let blr_addr = (opd_addr as u32) + 8;
    let mut bytes = [0u8; 16];
    bytes[0..4].copy_from_slice(&blr_addr.to_be_bytes());
    bytes[4..8].copy_from_slice(&0u32.to_be_bytes());
    bytes[8..12].copy_from_slice(&0x4e80_0020u32.to_be_bytes());
    let range = ByteRange::new(GuestAddr::new(opd_addr), 16)
        .expect("invariant: the OPD slot is a fixed 16-byte in-range address");
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
