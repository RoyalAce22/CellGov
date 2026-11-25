use super::hypercall_tests::{b, elf, li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::format::elf::ELF64_E_ENTRY;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};

/// The real address of the `sc` in [`window`].
const SC: u64 = 0x3008;

/// `ba target`.
///
/// [PPC-Book1 p:24 s:2.4] Branch I-form: AA is bit 30, and with AA = 1 the branch reaches EXTS(LI || 0b00).
fn ba(target: u64) -> u32 {
    b(i32::try_from(target).expect("a target below 32 MiB")) | 2
}

/// `bca 20, 0, target`: a conditional branch that branches always.
///
/// [PPC-Book1 p:8 s:1.7.2 B-Form] OPCD(0:5) BO(6:10) BI(11:15) BD(16:29) AA(30) LK(31).
/// [PPC-Book1 p:20 s:2.4.1] BO 1z1zz branches always.
fn bca(target: u64) -> u32 {
    let bd = u32::try_from(target).expect("a target below 32 KiB");
    assert!(bd < 0x8000, "a B-form displacement cannot reach {bd:#x}");
    (16 << 26) | (0b10100 << 21) | bd | 2
}

/// The words at real 0x3000: a window that loads 75 and runs `sc`, and
/// past its `blr` a path that loads `other` and branches to the real
/// address of that `sc`.
fn window(other: i16) -> [u32; 6] {
    [li(11, 75), NOP, PPC_SC_1, PPC_BLR, li(11, other), ba(SC)]
}

/// The ordinal of each site of `elf`, by address.
fn ordinals(elf: &[u8]) -> Vec<(u64, Option<u16>)> {
    scan_hypercalls(elf)
        .expect("the ELF scans")
        .into_iter()
        .map(|site| (site.address, site.ordinal))
        .collect()
}

/// An ELF whose one segment holds `words` from `vaddr`, with the entry
/// `entry`.
fn elf_with_entry(vaddr: u64, words: &[u32], entry: u64) -> Vec<u8> {
    let mut bytes = elf(vaddr, words, 4 * words.len() as u64);
    bytes[ELF64_E_ENTRY..ELF64_E_ENTRY + 8].copy_from_slice(&entry.to_be_bytes());
    bytes
}

#[test]
fn a_real_mode_absolute_branch_to_the_real_address_of_sc_adds_a_path_to_the_site() {
    // The ELF entry at real 0x2FFC runs in real mode and branches to the
    // path past the window, so that path runs in real mode.
    let site = |other: i16| {
        let mut words = vec![b(0x14)];
        words.extend(window(other));
        ordinals(&elf_with_entry(KERNEL + 0x2FFC, &words, KERNEL + 0x2FFC))
    };
    assert_eq!(site(76), [(KERNEL + SC, None)]);
    assert_eq!(site(75), [(KERNEL + SC, Some(75))]);
}

#[test]
fn a_word_that_a_real_mode_branch_reaches_at_its_real_address_also_runs_in_real_mode() {
    // The ELF entry at real 0x2FFC runs in real mode and branches to the
    // real address of the path past the window. No segment holds a
    // word at that address, so only the alias puts that path in real mode.
    let site = |other: i16| {
        let mut words = vec![ba(SC + 8)];
        words.extend(window(other));
        ordinals(&elf_with_entry(KERNEL + 0x2FFC, &words, KERNEL + 0x2FFC))
    };
    assert_eq!(site(76), [(KERNEL + SC, None)]);
    assert_eq!(site(75), [(KERNEL + SC, Some(75))]);
}

#[test]
fn a_real_mode_absolute_conditional_branch_to_the_real_address_of_sc_adds_a_path_to_the_site() {
    // The ELF entry at real 0x2FFC runs in real mode and branches to the
    // path past the window, so the `bca` there runs in real mode.
    let site = |other: i16| {
        let words = [
            b(0x14),
            li(11, 75),
            NOP,
            PPC_SC_1,
            PPC_BLR,
            li(11, other),
            bca(SC),
        ];
        ordinals(&elf_with_entry(KERNEL + 0x2FFC, &words, KERNEL + 0x2FFC))
    };
    assert_eq!(site(76), [(KERNEL + SC, None)]);
    assert_eq!(site(75), [(KERNEL + SC, Some(75))]);
}

#[test]
fn a_loop_in_real_mode_code_keeps_the_real_mode_path_to_the_site() {
    // The `b` past the `sc` goes back to the start of the window, so the
    // real-mode walk comes to the `sc` a second time.
    let site = |other: i16| {
        let words = [
            b(0x14),
            li(11, 75),
            NOP,
            PPC_SC_1,
            b(-0xC),
            li(11, other),
            ba(SC),
        ];
        ordinals(&elf_with_entry(KERNEL + 0x2FFC, &words, KERNEL + 0x2FFC))
    };
    assert_eq!(site(76), [(KERNEL + SC, None)]);
    assert_eq!(site(75), [(KERNEL + SC, Some(75))]);
}

#[test]
fn an_absolute_branch_at_the_elf_entry_reaches_the_word_at_its_real_address() {
    let site = |other: i16| {
        let words = window(other);
        ordinals(&elf_with_entry(KERNEL + 0x3000, &words, KERNEL + 0x3010))
    };
    assert_eq!(site(76), [(KERNEL + SC, None)]);
    assert_eq!(site(75), [(KERNEL + SC, Some(75))]);
}

#[test]
fn an_absolute_branch_that_only_a_named_address_reaches_adds_no_path() {
    // A doubleword in the file bytes names the path past the window, so
    // an indirect branch can enter it, but not in real mode.
    let named = KERNEL + 0x3010;
    let mut words = window(76).to_vec();
    words.extend([(named >> 32) as u32, named as u32]);
    let sites = ordinals(&elf(KERNEL + 0x3000, &words, 4 * words.len() as u64));
    assert_eq!(sites, [(KERNEL + SC, Some(75))]);
}
