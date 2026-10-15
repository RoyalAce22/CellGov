//! TLS-area placement after a title image or a firmware set that ends
//! past `0x10400000`.

use super::place_guest_heap;
use crate::prx::PrxLoadInfo;

/// ELF64 big-endian header plus one PT_LOAD at `(vaddr, memsz)`.
fn elf64_be_with_load(vaddr: u64, memsz: u64) -> Vec<u8> {
    const EHDR: usize = 64;
    const PHENT: usize = 56;
    const PT_LOAD: u32 = 1;
    let mut out = vec![0u8; EHDR + PHENT];
    out[..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    out[4] = 2; // ELFCLASS64
    out[5] = 2; // ELFDATA2MSB
    out[6] = 1; // EV_CURRENT
    out[18..20].copy_from_slice(&21u16.to_be_bytes()); // EM_PPC64
    out[32..40].copy_from_slice(&(EHDR as u64).to_be_bytes()); // e_phoff
    out[54..56].copy_from_slice(&(PHENT as u16).to_be_bytes()); // e_phentsize
    out[56..58].copy_from_slice(&1u16.to_be_bytes()); // e_phnum
    out[EHDR..EHDR + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    out[EHDR + 16..EHDR + 24].copy_from_slice(&vaddr.to_be_bytes());
    out[EHDR + 32..EHDR + 40].copy_from_slice(&memsz.to_be_bytes()); // p_filesz
    out[EHDR + 40..EHDR + 48].copy_from_slice(&memsz.to_be_bytes()); // p_memsz
    out
}

fn prx_ending_at(data_end: u64) -> PrxLoadInfo {
    PrxLoadInfo {
        name: "test.sprx".to_string(),
        stem: "test".to_string(),
        base: 0,
        data_end,
        toc: 0,
        relocs_applied: 0,
        module_start: None,
        module_stop: None,
    }
}

#[test]
fn a_title_image_past_0x10400000_gets_its_tls_area_after_its_data() {
    let elf = elf64_be_with_load(0x1000_0000, 0xa5_7708);
    let p = place_guest_heap(
        &elf,
        &[prx_ending_at(0x10b8_e100)],
        0x10a5_8000,
        &crate::NullSink,
    )
    .expect("an image past 0x10400000 places");
    assert_eq!(p.tls.base(), 0x10b9_0000);
    assert_eq!(p.tls.kernel_context_opd(), 0x10b9_fff0);
    assert_eq!(p.alloc_base, 0x10ba_0000);
}

#[test]
fn a_firmware_set_past_0x10400000_gets_the_tls_area_after_it() {
    let elf = elf64_be_with_load(0x1000_0000, 0x38_0570);
    let p = place_guest_heap(
        &elf,
        &[prx_ending_at(0x104e_dc38)],
        0x1038_1000,
        &crate::NullSink,
    )
    .expect("a firmware set past 0x10400000 places");
    assert_eq!(p.tls.base(), 0x104f_0000);
    assert_eq!(p.alloc_base, 0x1050_0000);
}
