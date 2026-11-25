use super::hypercall_constant_join_tests::{
    addi, bctr, build, descriptor, mtctr, skip, DESCRIPTORS, EVERY_UNRESOLVED, NOWHERE,
    WINDOW_RESOLVED,
};
use super::hypercall_ctr_decrement_tests::{quiet, window, SC, VECTOR};
use super::hypercall_entry_tests::{code, data_bytes, ordinals, pointers, word, Segment};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The window's `li r11`.
const WINDOW: u64 = SC - 8;

/// A word past the interrupt vectors, 0x7000 past the kernel base. No
/// constant of the cases names a word after it.
const AWAY: u64 = VECTOR + 0x6F00;

/// The ordinals with `words` at [`AWAY`], the window, the second window
/// and `data`. A `b` at [`VECTOR`] goes to [`AWAY`].
///
/// An interrupt can enter each word of the interrupt vectors with any
/// value in each GPR, so the cases run past the vectors.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
fn away_sites(words: &[u32], data: Segment) -> Vec<(u64, Option<u16>)> {
    let jump = word(PpuInstruction::B {
        offset: 0x6F00,
        aa: false,
        link: false,
    });
    let segments = [
        code(VECTOR, &[jump]),
        code(AWAY, words),
        window(&[]),
        quiet(),
        data,
    ];
    ordinals(0, &segments)
}

/// The ordinals with `ld r9,0(r3)`, then `step`, then `tail`, at
/// [`AWAY`]. A code pointer in the data names the window's `li r11`, so
/// the `ld` can load that address.
fn loaded_sites(step: &[u32], tail: &[u32]) -> Vec<(u64, Option<u16>)> {
    let mut words = vec![load(9)];
    words.extend_from_slice(step);
    words.extend_from_slice(tail);
    away_sites(&words, pointers(&[WINDOW]))
}

/// `ld rt,0(r3)`.
///
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
fn load(rt: u8) -> u32 {
    word(PpuInstruction::Ld { rt, ra: 3, imm: 0 })
}

fn mtlr(rs: u8) -> u32 {
    word(PpuInstruction::Mtlr { rs })
}

/// `mtctr r9; bctr`.
fn bctr_of_r9() -> [u32; 2] {
    [mtctr(9), bctr()]
}

/// The `addi` steps the loaded address of the window's `li r11` to its
/// `nop`, which the image does not name.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
#[ignore = "the census keeps no lost mark for a step of a loaded value yet, and the sound rule does not finish a held kernel in the time bound"]
fn a_bctr_of_a_loaded_address_plus_4_leaves_every_site_unresolved() {
    assert_eq!(
        loaded_sites(&[addi(9, 9, 4)], &bctr_of_r9()),
        EVERY_UNRESOLVED
    );
}

/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
#[ignore = "the census keeps no lost mark for a step of a loaded value yet, and the sound rule does not finish a held kernel in the time bound"]
fn a_blr_of_a_loaded_address_plus_4_leaves_every_site_unresolved() {
    assert_eq!(
        loaded_sites(&[addi(9, 9, 4)], &[mtlr(9), PPC_BLR]),
        EVERY_UNRESOLVED
    );
}

/// Each form is a step that the scan follows and that can give a value
/// other than its source.
///
/// [PPC-Book1 p:51 s:3.3.8] `addis` adds EXTS(SI || 0x0000) to RA.
/// [PPC-Book1 p:66 s:3.3.11] `ori` and `oris` OR RS with UI or UI || 0x0000 and place the result into RA.
/// [PPC-Book1 p:72 s:3.3.12.1] `rldicl` and `rldicr` rotate RS left by SH and AND it with MASK(MB, 63) or MASK(0, ME).
#[test]
#[ignore = "the census keeps no lost mark for a step of a loaded value yet, and the sound rule does not finish a held kernel in the time bound"]
fn a_bctr_of_a_loaded_address_that_each_followed_step_changes_leaves_every_site_unresolved() {
    let forms = [
        PpuInstruction::Addis {
            rt: 9,
            ra: 9,
            imm: 1,
        },
        PpuInstruction::Ori {
            ra: 9,
            rs: 9,
            imm: 4,
        },
        PpuInstruction::Oris {
            ra: 9,
            rs: 9,
            imm: 1,
        },
        PpuInstruction::Rldicl {
            ra: 9,
            rs: 9,
            sh: 4,
            mb: 0,
            rc: false,
        },
        PpuInstruction::Rldicr {
            ra: 9,
            rs: 9,
            sh: 0,
            me: 59,
            rc: false,
        },
    ];
    assert_eq!(forms.len(), 5);
    for form in forms {
        assert_eq!(crate::decode::decode(word(form)), Ok(form));
        assert_eq!(
            loaded_sites(&[word(form)], &bctr_of_r9()),
            EVERY_UNRESOLVED,
            "{form:?}"
        );
    }
}

/// Each form gives its source unchanged, so CTR holds the loaded address
/// of the window's `li r11`, which the image names.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:66 s:3.3.11] `ori` ORs RS with UI and places the result into RA.
/// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
/// [PPC-Book1 p:72 s:3.3.12.1] `rldicl` rotates RS left by SH and ANDs it with MASK(MB, 63).
#[test]
fn a_bctr_of_a_loaded_address_that_a_word_copies_unchanged_keeps_the_sites_resolved() {
    let forms = [
        PpuInstruction::Addi {
            rt: 9,
            ra: 9,
            imm: 0,
        },
        PpuInstruction::Ori {
            ra: 9,
            rs: 9,
            imm: 0,
        },
        PpuInstruction::Or {
            ra: 9,
            rs: 9,
            rb: 9,
            rc: false,
        },
        PpuInstruction::Rldicl {
            ra: 9,
            rs: 9,
            sh: 0,
            mb: 0,
            rc: false,
        },
    ];
    assert_eq!(forms.len(), 4);
    assert_eq!(loaded_sites(&[], &bctr_of_r9()), WINDOW_RESOLVED);
    for form in forms {
        assert_eq!(crate::decode::decode(word(form)), Ok(form));
        assert_eq!(
            loaded_sites(&[word(form)], &bctr_of_r9()),
            WINDOW_RESOLVED,
            "{form:?}"
        );
    }
}

/// The build writes r9 from 0 over the loaded address, so the `addi`
/// steps the built constant to a word that no segment holds.
///
/// [PPC-Book1 p:51 s:3.3.8] `addis` adds EXTS(SI || 0x0000) to 0 when RA is 0.
/// [PPC-Book1 p:66 s:3.3.11] `ori` ORs RS with UI and places the result into RA.
#[test]
fn a_bctr_of_a_constant_built_over_a_loaded_address_then_stepped_keeps_the_sites_resolved() {
    let mut step = build(9, NOWHERE).to_vec();
    step.push(addi(9, 9, 4));
    assert_eq!(loaded_sites(&step, &bctr_of_r9()), WINDOW_RESOLVED);
}

/// A descriptor names the word at [`AWAY`] with the TOC 0x5000, so r2
/// holds that TOC on each word of the function, also after the call.
/// The `addi` steps the TOC to a word that no segment holds.
///
/// [OFW-4.93 lv2_kernel a:0x80000000002765e4-0x8000000000276600] A call through a descriptor loads LR from its first doubleword and r2 from its second.
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
#[test]
fn a_bctr_of_the_toc_in_r2_stepped_after_a_call_keeps_the_sites_resolved() {
    let call = word(PpuInstruction::B {
        offset: 16,
        aa: false,
        link: true,
    });
    let words = [call, addi(9, 2, 0x10), mtctr(9), bctr(), PPC_BLR];
    let toc = data_bytes(DESCRIPTORS, descriptor(AWAY, u64::from(NOWHERE)));
    assert_eq!(away_sites(&words, toc), WINDOW_RESOLVED);
}

/// The `blt` brings r9 with the loaded address of the window's `li r11`
/// to the `addi`, and the build falls through to it with a constant. So
/// the `addi` can step that address to the window's `nop`.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
#[test]
#[ignore = "a step of a register that holds a constant and can also hold a loaded value keeps only the stepped constant"]
fn a_bctr_of_a_constant_joined_with_a_loaded_address_then_stepped_leaves_every_site_unresolved() {
    let mut words = vec![load(9), skip(2)];
    words.extend(build(9, NOWHERE));
    words.push(addi(9, 9, 4));
    words.extend(bctr_of_r9());
    assert_eq!(away_sites(&words, pointers(&[WINDOW])), EVERY_UNRESOLVED);
}
