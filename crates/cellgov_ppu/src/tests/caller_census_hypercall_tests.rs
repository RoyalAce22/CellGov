use super::*;
use cellgov_ps3_abi::format::elf::{
    ELFCLASS64, ELFDATA2MSB, ELF_EI_CLASS, ELF_EI_DATA, ELF_HEADER_SIZE, ELF_MAGIC, ELF_PHENTSIZE,
    ELF_PHENTSIZE_OFFSET, ELF_PHNUM_OFFSET, ELF_PHOFF_OFFSET, PF_X, PHDR_P_FILESZ_OFFSET,
    PHDR_P_MEMSZ_OFFSET, PHDR_P_OFFSET_OFFSET, PHDR_P_VADDR_OFFSET, PT_LOAD,
};

/// The kernel's link address.
pub(super) const KERNEL: u64 = 0x8000_0000_0000_0000;

/// `sc` with LEV = 0.
///
/// [PPC-Book1 p:26 s:2.4.2] sc LEV-field encoding.
const SC_0: u32 = 0x4400_0002;

/// `sc` with LEV = 1.
///
/// [PPC-Book1 p:26 s:2.4.2] sc LEV-field encoding.
const SC_1: u32 = 0x4400_0022;

/// `li rt, value`.
///
/// [PPC-Book1 p:162 s:B.9 Load Immediate] li Rx,value == addi Rx,0,value.
pub(super) fn li(rt: u8, value: i16) -> u32 {
    (14 << 26) | (u32::from(rt) << 21) | u32::from(value as u16)
}

/// An ELF with one executable segment at `vaddr` that holds `words`, and
/// `memsz` memory bytes.
pub(super) fn elf(vaddr: u64, words: &[u32], memsz: u64) -> Vec<u8> {
    const PAYLOAD: usize = ELF_HEADER_SIZE + ELF_PHENTSIZE;
    let filesz = 4 * words.len() as u64;
    let mut elf = vec![0; PAYLOAD];
    elf[0..4].copy_from_slice(&ELF_MAGIC);
    elf[ELF_EI_CLASS] = ELFCLASS64;
    elf[ELF_EI_DATA] = ELFDATA2MSB;
    elf[6] = 1;
    elf[18..20].copy_from_slice(&21u16.to_be_bytes());
    let mut put = |at: usize, bytes: &[u8]| elf[at..at + bytes.len()].copy_from_slice(bytes);
    put(ELF_PHOFF_OFFSET, &(ELF_HEADER_SIZE as u64).to_be_bytes());
    put(ELF_PHENTSIZE_OFFSET, &(ELF_PHENTSIZE as u16).to_be_bytes());
    put(ELF_PHNUM_OFFSET, &1u16.to_be_bytes());
    put(ELF_HEADER_SIZE, &PT_LOAD.to_be_bytes());
    put(ELF_HEADER_SIZE + 4, &PF_X.to_be_bytes());
    put(
        ELF_HEADER_SIZE + PHDR_P_OFFSET_OFFSET,
        &(PAYLOAD as u64).to_be_bytes(),
    );
    put(ELF_HEADER_SIZE + PHDR_P_VADDR_OFFSET, &vaddr.to_be_bytes());
    put(
        ELF_HEADER_SIZE + PHDR_P_FILESZ_OFFSET,
        &filesz.to_be_bytes(),
    );
    put(ELF_HEADER_SIZE + PHDR_P_MEMSZ_OFFSET, &memsz.to_be_bytes());
    elf.extend(words.iter().flat_map(|w| w.to_be_bytes()));
    elf
}

#[test]
fn a_kernel_hypercall_site_resolves_its_number_and_an_lv2_site_is_not_one() {
    // The segment lies above the real pages of the interrupt vectors.
    let words = [li(11, 74), SC_1, li(11, 22), SC_0, li(3, 1), SC_1];
    let sites = scan_hypercalls(&elf(KERNEL + 0x1_0000, &words, 24)).expect("the ELF scans");
    assert_eq!(
        sites,
        [
            CallerSite {
                address: KERNEL + 0x1_0004,
                ordinal: Some(74),
            },
            CallerSite {
                address: KERNEL + 0x1_0014,
                ordinal: None,
            },
        ]
    );
}

/// `bc BO, 30, +8` with link `lk`. BI 30 is the EQ bit of CR field 7.
///
/// [PPC-Book1 p:8 s:1.7.2 B-Form] OPCD(0:5) BO(6:10) BI(11:15) BD(16:29) AA(30) LK(31).
fn bc(bo: u32, lk: u32) -> u32 {
    (16 << 26) | (bo << 21) | (30 << 16) | 8 | lk
}

#[test]
fn a_conditional_branch_keeps_a_hypercall_number_and_not_an_lv2_ordinal() {
    // [PPC-Book1 p:20 s:2.4.1] BO 001at: branch if the CR bit is 0, so
    // `bne cr7` falls through when the bit is set. BO 1z1zz branches always.
    let words = [li(11, 105), bc(0b00100, 0), SC_1];
    let sites = scan_hypercalls(&elf(KERNEL, &words, 12)).expect("the ELF scans");
    assert_eq!(sites[0].ordinal, Some(105));
    for middle in [bc(0b00100, 1), bc(0b10100, 0)] {
        let sites = scan_hypercalls(&elf(KERNEL, &[li(11, 105), middle, SC_1], 12)).expect("scans");
        assert_eq!(sites[0].ordinal, None, "{middle:#x}");
    }
    let lv2 = scan_words(0, &[li(11, 22), bc(0b00100, 0), SC_0]);
    assert_eq!(lv2[0].ordinal, None, "the LV2 scan keeps its rule");
}

pub(super) const NOP: u32 = 0x6000_0000;

/// `b` with a byte displacement of `offset`.
pub(super) fn b(offset: i32) -> u32 {
    (18 << 26) | (offset as u32 & 0x03FF_FFFC)
}

#[test]
fn a_branch_from_outside_the_window_into_it_leaves_a_hypercall_site_unresolved() {
    // The taken path of the bne skips `li r11, 75` and reaches sc with 74.
    let skipped = [li(11, 74), bc(0b00100, 0), li(11, 75), SC_1];
    // The b after sc re-enters the window at sc with any r11.
    let reentered = [li(11, 75), NOP, SC_1, b(-4)];
    for words in [&skipped[..], &reentered[..]] {
        let sites = scan_hypercalls(&elf(KERNEL, words, 16)).expect("the ELF scans");
        assert_eq!(sites[0].ordinal, None, "{words:x?}");
    }
    // A branch from inside the window keeps the number.
    let inside = [li(11, 75), bc(0b00100, 0), NOP, SC_1];
    let sites = scan_hypercalls(&elf(KERNEL, &inside, 16)).expect("the ELF scans");
    assert_eq!(sites[0].ordinal, Some(75));
}

/// [PPC-Book1 p:24 s:2.4] Branch I-form: NIA <- EXTS(LI||0b00) when AA, else CIA+EXTS(LI||0b00).
#[test]
fn an_absolute_branch_into_the_window_leaves_a_hypercall_site_unresolved() {
    const LOW: u64 = 0x1_0000;
    // `ba LOW + 8` after sc re-enters the window at sc.
    let words = [li(11, 75), NOP, SC_1, b(LOW as i32 + 8) | 2];
    let sites = scan_hypercalls(&elf(LOW, &words, 16)).expect("the ELF scans");
    assert_eq!(
        sites,
        [CallerSite {
            address: LOW + 8,
            ordinal: None,
        }]
    );
}

/// A branch onto `li r11` itself runs the load, so no path skips it.
#[test]
fn a_branch_onto_the_load_keeps_a_hypercall_site_resolved() {
    const LOW: u64 = 0x1_0000;
    let words = [b(4), li(11, 75), SC_1];
    let sites = scan_hypercalls(&elf(LOW, &words, 12)).expect("the ELF scans");
    assert_eq!(
        sites,
        [CallerSite {
            address: LOW + 8,
            ordinal: Some(75),
        }]
    );
}

#[test]
fn a_hypercall_number_past_the_lv2_table_still_resolves() {
    let number = SYSCALL_TABLE_SLOTS as i16;
    let sites = scan_hypercalls(&elf(KERNEL, &[li(11, number), SC_1], 8)).expect("the ELF scans");
    assert_eq!(sites[0].ordinal, Some(number as u16));
}

#[test]
fn a_hypercall_scan_refuses_a_segment_with_more_file_bytes_or_past_the_address_space() {
    assert!(matches!(
        scan_hypercalls(&elf(KERNEL, &[SC_1, SC_1], 4)),
        Err(CallerScanError::SegmentFileszExceedsMemsz { index: 0, .. })
    ));
    assert!(matches!(
        scan_hypercalls(&elf(u64::MAX - 3, &[SC_1], 8)),
        Err(CallerScanError::SegmentOutOfRange { index: 0 })
    ));
}
