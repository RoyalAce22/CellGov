use super::hypercall_tests::{b, elf, li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};

/// `bne cr7` with a byte displacement of `offset`.
///
/// [PPC-Book1 p:8 s:1.7.2 B-Form] OPCD(0:5) BO(6:10) BI(11:15) BD(16:29) AA(30) LK(31).
/// [PPC-Book1 p:20 s:2.4.1] BO 001at: branch if the CR bit is 0; BI 30 is the EQ bit of CR field 7.
fn bne_cr7(offset: i16) -> u32 {
    (16 << 26) | (0b00100 << 21) | (30 << 16) | u32::from(offset as u16 & 0xFFFC)
}

/// The sites of one kernel segment that holds `words`.
fn sites(words: &[u32]) -> Vec<CallerSite> {
    let memsz = 4 * words.len() as u64;
    scan_hypercalls(&elf(KERNEL, words, memsz)).expect("the ELF scans")
}

/// The site at word `index` of a kernel segment.
fn site(index: u64, ordinal: Option<u16>) -> CallerSite {
    CallerSite {
        address: KERNEL + 4 * index,
        ordinal,
    }
}

/// The word "DIR/" of a path string in the kernel's read-only data
/// [OFW-4.93 lv2_kernel a:0x80000000002d7cb4].
const STRING_WORD: u32 = 0x4449_522F;

#[test]
fn a_string_word_that_decodes_as_sc_1_is_not_a_site() {
    assert_eq!(
        crate::decode::decode(STRING_WORD).ok(),
        Some(PpuInstruction::Sc { lev: 1 })
    );
    assert_eq!(
        sites(&[li(11, 74), STRING_WORD, li(11, 74), PPC_SC_1]),
        [site(3, Some(74))]
    );
}

/// Shape A: the `sc` block sits after the epilogue's `blr`, and a
/// conditional branch reaches it
/// [OFW-4.93 lv2_kernel a:0x80000000000087a0-0x80000000000087f8].
#[test]
fn an_sc_block_after_the_epilogue_blr_resolves_its_number() {
    let words = [
        li(11, 12),
        li(3, -1),
        bne_cr7(12),
        NOP,
        PPC_BLR,
        NOP,
        PPC_SC_1,
    ];
    assert_eq!(sites(&words), [site(6, Some(12))]);
}

/// Shape A with a `nop` after `blr` that pads the `sc` block, which no
/// path reaches [OFW-2.43 lv2_kernel a:0x800000000001e590-0x800000000001e5e0].
#[test]
fn an_sc_block_after_blr_and_padding_resolves_its_number() {
    let words = [li(11, 231), bne_cr7(16), NOP, PPC_BLR, NOP, NOP, PPC_SC_1];
    assert_eq!(sites(&words), [site(6, Some(231))]);
}

/// Shape B: `li r11` is 19 words above `sc`, past the old 12-word
/// lookback [OFW-4.93 lv2_kernel a:0x8000000000119578-0x80000000001195c4].
#[test]
fn an_li_r11_hoisted_19_words_above_sc_resolves_its_number() {
    let mut words = vec![li(11, 228)];
    words.extend([NOP; 18]);
    words.push(PPC_SC_1);
    assert_eq!(words.len(), 20);
    assert_eq!(sites(&words), [site(19, Some(228))]);
}

/// Shape C: two paths each load the number and join before `sc`
/// [OFW-4.93 lv2_kernel a:0x80000000000a238c-0x80000000000a23fc].
#[test]
fn two_paths_that_load_the_same_number_resolve_it_and_two_numbers_do_not() {
    let joined = |second: i16| [bne_cr7(12), li(11, 194), b(8), li(11, second), PPC_SC_1];
    assert_eq!(sites(&joined(194)), [site(4, Some(194))]);
    assert_eq!(sites(&joined(195)), [site(4, None)]);
}
