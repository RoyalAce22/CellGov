//! What TLS pre-init refuses, and what the reservation holds after it.

use super::*;

use cellgov_ps3_abi::format::elf::{
    ELF_EI_CLASS, ELF_EI_DATA, ELF_MAGIC, ELF_PHENTSIZE, ELF_PHENTSIZE_OFFSET, ELF_PHNUM_OFFSET,
    ELF_PHOFF_OFFSET, PHDR_P_FILESZ_OFFSET, PHDR_P_MEMSZ_OFFSET, PHDR_P_VADDR_OFFSET, PT_TLS,
};

use crate::NullSink;

/// Big-endian ELF64 header carrying exactly one PT_TLS program header.
fn elf_with_pt_tls(vaddr: u64, filesz: u64, memsz: u64) -> Vec<u8> {
    let phoff = cellgov_ps3_abi::format::elf::ELF_HEADER_SIZE;
    let mut buf = vec![0u8; phoff + ELF_PHENTSIZE];
    buf[0..ELF_MAGIC.len()].copy_from_slice(&ELF_MAGIC);
    buf[ELF_EI_CLASS] = 2; // ELFCLASS64
    buf[ELF_EI_DATA] = 2; // ELFDATA2MSB
    buf[ELF_PHOFF_OFFSET..ELF_PHOFF_OFFSET + 8].copy_from_slice(&(phoff as u64).to_be_bytes());
    buf[ELF_PHENTSIZE_OFFSET..ELF_PHENTSIZE_OFFSET + 2]
        .copy_from_slice(&(ELF_PHENTSIZE as u16).to_be_bytes());
    buf[ELF_PHNUM_OFFSET..ELF_PHNUM_OFFSET + 2].copy_from_slice(&1u16.to_be_bytes());
    let pv = phoff + PHDR_P_VADDR_OFFSET;
    let pf = phoff + PHDR_P_FILESZ_OFFSET;
    let pm = phoff + PHDR_P_MEMSZ_OFFSET;
    buf[phoff..phoff + 4].copy_from_slice(&PT_TLS.to_be_bytes());
    buf[pv..pv + 8].copy_from_slice(&vaddr.to_be_bytes());
    buf[pf..pf + 8].copy_from_slice(&filesz.to_be_bytes());
    buf[pm..pm + 8].copy_from_slice(&memsz.to_be_bytes());
    buf
}

fn tls_refusal(vaddr: u64, filesz: u64, memsz: u64) -> TlsError {
    let elf = elf_with_pt_tls(vaddr, filesz, memsz);
    let mut mem = GuestMemory::new(0x1000);
    pre_init_tls(&elf, &mut mem, &NullSink).expect_err("refusal expected")
}

fn place(mem: &mut GuestMemory, addr: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).expect("in-range");
    mem.apply_commit(range, bytes).expect("commit");
}

#[test]
fn a_pt_tls_claiming_more_file_bytes_than_memory_bytes_is_refused() {
    assert!(
        matches!(
            tls_refusal(0, 0x40, 0x10),
            TlsError::TemplateLongerThanImage {
                filesz: 0x40,
                memsz: 0x10
            }
        ),
        "filesz past memsz must be named, not indexed past the staging buffer"
    );
}

#[test]
fn a_pt_tls_memsz_that_overflows_the_reservation_offset_is_refused() {
    assert!(
        matches!(tls_refusal(0, 0, u64::MAX), TlsError::DestOverflow { .. }),
        "a memsz that overflows the offset arithmetic must be named"
    );
}

#[test]
fn a_pt_tls_reaching_the_kernel_context_opd_slot_is_refused() {
    // The reservation is 64 KB and the OPD owns its last 16 bytes. So
    // the first memsz whose 0x30-offset template touches the slot is
    // 0x10000 - 0x10 - 0x30 + 1.
    let memsz = 0xFFF0 - 0x30 + 1;
    assert!(
        matches!(
            tls_refusal(0, 0, memsz),
            TlsError::TemplateOverlapsOpd {
                opd_offset: 0xFFF0,
                ..
            }
        ),
        "a template reaching the OPD slot must be named, not silently clipped"
    );
    // One byte less clears the slot and reaches the memory bound.
    assert!(matches!(
        tls_refusal(0, 0, memsz - 1),
        TlsError::DestOutOfRange { .. }
    ));
}

#[test]
fn a_pt_tls_sourced_past_the_end_of_guest_memory_is_refused() {
    assert!(
        matches!(
            tls_refusal(0x1000, 0x10, 0x10),
            TlsError::SourceOutOfRange {
                mem_len: 0x1000,
                ..
            }
        ),
        "the template head must be read from mapped memory"
    );
}

#[test]
fn a_well_formed_pt_tls_reaches_the_guest_memory_bound() {
    assert!(
        matches!(tls_refusal(0, 0x10, 0x10), TlsError::DestOutOfRange { .. }),
        "filesz == memsz is well formed and must pass to the bounds check"
    );
}

#[test]
fn a_pt_tls_with_file_bytes_but_no_memory_image_is_refused_not_skipped() {
    assert!(
        matches!(
            tls_refusal(0, 0x10, 0),
            TlsError::TemplateLongerThanImage {
                filesz: 0x10,
                memsz: 0
            }
        ),
        "the zero-memsz skip must not absorb a header that declares file bytes"
    );
}

#[test]
fn a_zero_memsz_pt_tls_initializes_nothing() {
    let elf = elf_with_pt_tls(0, 0, 0);
    let mut mem = GuestMemory::new(0x1000);
    assert!(pre_init_tls(&elf, &mut mem, &NullSink).is_ok());
}

#[test]
fn pre_init_zeroes_the_header_gap_and_the_bss_tail_with_the_template() {
    const VADDR: u64 = 0x1000;
    let elf = elf_with_pt_tls(VADDR, 8, 0x10);
    let mut mem = GuestMemory::new(TLS_BASE as usize + 0x40);
    place(&mut mem, VADDR, &[0x11u8; 8]);
    place(&mut mem, TLS_BASE, &[0xAAu8; 0x30]);

    pre_init_tls(&elf, &mut mem, &NullSink).expect("a well-formed PT_TLS initializes");

    let base = TLS_BASE as usize;
    let bytes = mem.as_bytes();
    assert!(
        bytes[base..base + 0x30].iter().all(|b| *b == 0),
        "the per-thread header gap must not carry pre-boot bytes"
    );
    assert_eq!(
        &bytes[base + 0x30..base + 0x38],
        &[0x11u8; 8],
        "the template head lands 0x30 above the reservation base"
    );
    assert!(
        bytes[base + 0x38..base + 0x40].iter().all(|b| *b == 0),
        "the BSS tail past filesz must be zero"
    );
}
