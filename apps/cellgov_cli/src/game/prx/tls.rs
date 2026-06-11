//! TLS pre-init from the ELF's PT_TLS segment and the synthetic
//! kernel-context OPD that liblv2's entry expects in r11 / r12.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

use crate::cli::exit::die;
use crate::game::boot::HLE_HEAP_BASE;

/// Must match the firmware sysPrxForUser `sys_initialize_tls`
/// allocation seen by liblv2 at module_start time.
pub(in crate::game) const TLS_BASE: u64 = 0x10400000;

/// Guest address of the synthetic kernel-context OPD installed by
/// [`install_kernel_context_opd`]. Sits in the last 16 bytes of the
/// 64 KB TLS reservation, immediately below `HLE_HEAP_BASE`.
const KERNEL_CTX_OPD_ADDR: u64 = 0x1040_FFF0;

const _: () = assert!(KERNEL_CTX_OPD_ADDR > TLS_BASE);
const _: () = assert!(KERNEL_CTX_OPD_ADDR + 16 == HLE_HEAP_BASE as u64);

/// Offset (from `TLS_BASE`) at which the PT_TLS template starts; PS3
/// kernel convention leaves `0x30` bytes of scratch for the per-thread
/// TLS header that the runtime writes via `sys_initialize_tls`.
const TLS_TEMPLATE_OFFSET: u64 = 0x30;

/// Pre-initialize TLS from the ELF's PT_TLS segment.
///
/// Stages the template bytes and any BSS tail into a single buffer,
/// then commits with one `apply_commit` so the guest never observes a
/// partially initialized TLS image. PS3 LV2 performs this during
/// process creation before any module_start runs.
pub(in crate::game) fn pre_init_tls(elf_data: &[u8], mem: &mut GuestMemory) {
    let tls = match cellgov_ppu::loader::find_tls_segment(elf_data) {
        Some(t) => t,
        None => return,
    };

    let p_vaddr = tls.vaddr as usize;
    let p_filesz = tls.filesz as usize;
    let p_memsz = tls.memsz as usize;
    if p_memsz == 0 {
        return;
    }

    // Reject a PT_TLS that would extend into the kernel-context OPD
    // slot at the top of the reservation: the OPD commit happens later
    // and would clobber the template tail otherwise.
    let opd_offset = (KERNEL_CTX_OPD_ADDR - TLS_BASE) as usize;
    let template_end_offset = TLS_TEMPLATE_OFFSET as usize + p_memsz;
    if template_end_offset > opd_offset {
        die(&format!(
            "tls: PT_TLS memsz=0x{p_memsz:x} extends past offset 0x{opd_offset:x} \
             (kernel-context OPD slot); shrink the template or relocate the OPD"
        ));
    }

    let m_len = mem.as_bytes().len();
    let tls_data_start = TLS_BASE as usize + TLS_TEMPLATE_OFFSET as usize;

    let src_end = p_vaddr.checked_add(p_filesz).unwrap_or_else(|| {
        die(&format!(
            "tls: PT_TLS vaddr=0x{p_vaddr:x} + filesz=0x{p_filesz:x} overflows usize"
        ))
    });
    if src_end > m_len {
        die(&format!(
            "tls: PT_TLS src 0x{p_vaddr:x}+0x{p_filesz:x} exceeds guest memory ({m_len} bytes)"
        ));
    }
    let dst_end = tls_data_start.checked_add(p_memsz).unwrap_or_else(|| {
        die(&format!(
            "tls: TLS dst 0x{tls_data_start:x} + memsz=0x{p_memsz:x} overflows usize"
        ))
    });
    if dst_end > m_len {
        die(&format!(
            "tls: TLS dst 0x{tls_data_start:x}+0x{p_memsz:x} exceeds guest memory ({m_len} bytes)"
        ));
    }

    let mut image = vec![0u8; p_memsz];
    if p_filesz > 0 {
        let m = mem.as_bytes();
        image[..p_filesz].copy_from_slice(&m[p_vaddr..src_end]);
    }
    let range = ByteRange::new(GuestAddr::new(tls_data_start as u64), p_memsz as u64)
        .unwrap_or_else(|| die(&format!("tls: invalid byte range at 0x{tls_data_start:x}")));
    mem.apply_commit(range, &image).unwrap_or_else(|e| {
        die(&format!(
            "tls: pre-init commit at 0x{tls_data_start:x} FAILED ({e:?}); TLS not initialized"
        ))
    });

    println!(
        "tls: pre-initialized from PT_TLS at 0x{:x} (filesz=0x{:x}, memsz=0x{:x}) -> 0x{:x}",
        p_vaddr, p_filesz, p_memsz, TLS_BASE
    );
}

/// Write a `{code, toc}` OPD whose body is a single `blr` and return
/// its address. liblv2's entry expects kernel-side function OPDs in
/// r11 / r12; the synthetic OPD lets those calls return cleanly.
pub(in crate::game) fn install_kernel_context_opd(mem: &mut GuestMemory) -> u64 {
    let opd_addr = KERNEL_CTX_OPD_ADDR;
    let blr_addr = (opd_addr as u32) + 8;
    let mut bytes = [0u8; 16];
    bytes[0..4].copy_from_slice(&blr_addr.to_be_bytes());
    bytes[4..8].copy_from_slice(&0u32.to_be_bytes());
    bytes[8..12].copy_from_slice(&0x4e80_0020u32.to_be_bytes());
    let range = ByteRange::new(GuestAddr::new(opd_addr), 16).expect("range");
    if let Err(e) = mem.apply_commit(range, &bytes) {
        die(&format!(
            "module_start: kernel-context OPD install at 0x{opd_addr:x} FAILED ({e:?}); \
             liblv2 module_start would fault on the entry r11/r12 path"
        ));
    }
    opd_addr
}
