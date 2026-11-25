use super::hypercall_constant_join_tests::{
    addi, bctr, build, descriptor, mflr, mtctr, skip, writers, DESCRIPTORS, EVERY_UNRESOLVED,
    NOWHERE, WINDOW_RESOLVED,
};
use super::hypercall_ctr_decrement_tests::{bc, quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_entry_tests::{code, data_bytes, ordinals, word};
use super::hypercall_join_drop_tests::RAISED_LIMIT;
use super::hypercall_tests::NOP;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The ordinals when an indirect branch can enter the window's `sc` and
/// no word of the second window after its `li r11`.
const SC_ENTERED: [(u64, Option<u16>); 2] = [(SC, None), (QUIET_SC, Some(77))];

/// A word past the interrupt vectors, 0x7000 past the kernel base. No
/// constant of the cases names a word after it, and no load of their
/// file bytes gives one: an indirect branch can enter each such word.
const AWAY: u64 = VECTOR + 0x6F00;

/// The ordinals with `words` at [`AWAY`], the window and the second
/// window. A `b` at [`VECTOR`] goes to [`AWAY`].
///
/// An interrupt can enter each word of the interrupt vectors with any
/// value in each GPR, so the cases run their loop past the vectors.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
fn away_sites(words: &[u32]) -> Vec<(u64, Option<u16>)> {
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
    ];
    ordinals(0, &segments)
}

/// `li r3,base; loop: addi r3,r3,step; blt loop`.
///
/// Each word that a class of the loop names is a word that an indirect
/// branch can enter with any value in r3. So the cases that resolve keep
/// each word from the `addi` to the `mtctr` or `mtlr` off the class.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0.
fn stepped_loop(base: u16, step: i16) -> Vec<u32> {
    vec![
        addi(3, 0, base.cast_signed()),
        addi(3, 3, step),
        bc(0b01100, -4),
    ]
}

/// `bl` to the word `offset` bytes on.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
fn bl(offset: i32) -> u32 {
    word(PpuInstruction::B {
        offset,
        aa: false,
        link: true,
    })
}

fn mtlr(rs: u8) -> u32 {
    word(PpuInstruction::Mtlr { rs })
}

fn mfctr(rt: u8) -> u32 {
    word(PpuInstruction::Mfctr { rt })
}

/// `or rA,rS,rS`, which copies rS into rA.
///
/// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
fn copy(ra: u8, rs: u8) -> u32 {
    word(PpuInstruction::Or {
        ra,
        rs,
        rb: rs,
        rc: false,
    })
}

/// The loop adds 0x10 to r3 from 0x5000, so r3 holds only multiples of
/// 0x10. The bound names each word at a multiple of 0x10: the `li r11`
/// of each window, and no word after it.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_a_loop_steps_by_0x10_from_0x5000_enters_no_word_after_a_li_r11() {
    let mut words = stepped_loop(NOWHERE, 0x10);
    words.extend([mtctr(3), bctr()]);
    assert_eq!(away_sites(&words), WINDOW_RESOLVED);
}

/// The loop adds 0x10 to r3 from 0x5008, so the bound names each word
/// at 8 past a multiple of 0x10, the window's `sc` among them.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_a_loop_steps_by_0x10_from_0x5008_enters_the_window_sc() {
    let mut words = vec![NOP, NOP];
    words.extend(stepped_loop(NOWHERE + 8, 0x10));
    words.extend([mtctr(3), bctr()]);
    assert_eq!(away_sites(&words), SC_ENTERED);
}

/// The loop adds 0x20 to r3 from 0x5000, so the bound names each word
/// at a multiple of 0x20, and no word from the `addi` to the `blr`.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_of_a_gpr_that_a_loop_steps_by_0x20_from_0x5000_enters_no_word_after_a_li_r11() {
    let mut words = stepped_loop(NOWHERE, 0x20);
    words.extend([mtlr(3), PPC_BLR]);
    assert_eq!(away_sites(&words), WINDOW_RESOLVED);
}

/// The `or` copies r3 into r5, and the `addi` adds the step of the loop
/// once more, so r5 holds only multiples of 0x20.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_copy_of_a_loop_gpr_that_adds_the_loop_step_enters_no_word_after_a_li_r11() {
    let mut words = stepped_loop(NOWHERE, 0x20);
    words.extend([copy(5, 3), addi(5, 5, 0x20), mtctr(5), bctr()]);
    assert_eq!(away_sites(&words), WINDOW_RESOLVED);
}

/// The `blt` at word 1 goes to the loop at word 3, and the `ld` falls
/// through to it, so a path brings r3 with a loaded value that the scan
/// holds nothing for. The loop adds 0x20 to that value too, which can
/// leave r3 off the class of the constants.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_loop_gpr_that_a_path_with_a_loaded_value_joins_leaves_every_site_unresolved() {
    let load = word(PpuInstruction::Ld {
        rt: 3,
        ra: 4,
        imm: 0,
    });
    let words = [
        addi(3, 0, NOWHERE.cast_signed()),
        bc(0b01100, 8),
        load,
        addi(3, 3, 0x20),
        bc(0b01100, -4),
        mtctr(3),
        bctr(),
    ];
    assert_eq!(away_sites(&words), EVERY_UNRESOLVED);
}

/// The `bl` calls the routine at word 6, which adds 8 to r3 and returns.
/// The scan does not join what the routine leaves in r3 back into the
/// `mtctr`, so r3 there can hold a value off the class of the loop.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_loop_gpr_after_a_call_whose_routine_steps_it_leaves_every_site_unresolved() {
    let mut words = stepped_loop(NOWHERE, 0x20);
    words.extend([bl(12), mtctr(3), bctr(), addi(3, 3, 8), PPC_BLR]);
    assert_eq!(away_sites(&words), EVERY_UNRESOLVED);
}

/// The `nop` puts the `mtctr` at 0x10 past [`AWAY`], a word of the class
/// of the loop. An indirect branch can enter it with any value in r3.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_loop_gpr_whose_mtctr_is_a_word_of_the_class_leaves_every_site_unresolved() {
    let mut words = stepped_loop(NOWHERE, 0x10);
    words.extend([NOP, mtctr(3), bctr()]);
    assert_eq!(away_sites(&words), EVERY_UNRESOLVED);
}

/// The scan does not follow the `divw`. It keeps the mark of r3, which
/// the word can leave unchanged, and takes r3 to also hold a value that
/// it holds nothing for.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_loop_gpr_that_a_divw_can_write_leaves_every_site_unresolved() {
    let divide = word(PpuInstruction::Divw {
        rt: 3,
        ra: 3,
        rb: 4,
        oe: false,
        rc: false,
    });
    let mut words = stepped_loop(NOWHERE, 0x20);
    words.extend([divide, mtctr(3), bctr()]);
    assert_eq!(away_sites(&words), EVERY_UNRESOLVED);
}

/// The `addi` adds 8, which is not the step of the loop, to r3.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_loop_gpr_plus_another_constant_leaves_every_site_unresolved() {
    let mut words = stepped_loop(NOWHERE, 0x10);
    words.extend([addi(5, 3, 8), mtctr(5), bctr()]);
    assert_eq!(away_sites(&words), EVERY_UNRESOLVED);
}

/// The first loop steps r3 by 0x10, and the `blt` at word 3 skips the
/// second loop, which steps r3 by 0x20, to the `mtctr` at word 7. So the
/// `mtctr` joins two progressions with different steps.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_join_of_two_loops_with_different_steps_leaves_every_site_unresolved() {
    let mut words = stepped_loop(NOWHERE, 0x10);
    words.push(bc(0b01100, 4 * 4));
    words.extend(stepped_loop(NOWHERE, 0x20));
    words.extend([mtctr(3), bctr()]);
    assert_eq!(away_sites(&words), EVERY_UNRESOLVED);
}

/// The ordinals with `tail` after `mtctr r5`, the window, the second
/// window and 65 descriptors for the word at [`AWAY`]. Before the
/// `mtctr`, an `or` copies r2 into r5, a loop adds 0x20 to r5 on each
/// pass, and a `b` over a `nop` goes to the `mtctr`. No word from the `addi`
/// to the `mtctr` is a word of the class, a multiple of 0x20.
///
/// Each descriptor gives its own TOC, a multiple of 0x20. So r2 holds
/// 65 TOCs, and the one path from the `or` brings more constants to the
/// loop join than a raised bound keeps. The join keeps no constant of
/// r5. The `b` brings r5 to the `mtctr`, so no path of fall-throughs
/// alone brings a constant of r5 there. CTR from r5 holds only the
/// values of the class.
///
/// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
fn toc_loop_sites(tail: &[u32]) -> Vec<(u64, Option<u16>)> {
    let over = word(PpuInstruction::B {
        offset: 8,
        aa: false,
        link: false,
    });
    let mut words = vec![
        copy(5, 2),
        addi(5, 5, 0x20),
        bc(0b01100, -4),
        over,
        NOP,
        mtctr(5),
    ];
    words.extend_from_slice(tail);
    let tocs = (0..=RAISED_LIMIT)
        .flat_map(|toc| descriptor(AWAY, u64::from(NOWHERE + 0x20 * toc)))
        .collect();
    let segments = [
        code(AWAY, &words),
        window(&[]),
        quiet(),
        data_bytes(DESCRIPTORS, tocs),
    ];
    ordinals(0, &segments)
}

/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_ctr_that_holds_only_values_of_a_class_enters_no_word_after_a_li_r11() {
    assert_eq!(toc_loop_sites(&[bctr()]), WINDOW_RESOLVED);
}

/// The `bdz` decrements a CTR that holds only values of the class, and
/// goes to the `bctr` on both edges.
///
/// [PPC-Book1 p:24 s:2.4] A `bc` with BO bit 2 at 0 decrements CTR.
/// [PPC-Book1 p:20 s:2.4.1] BO 1a01t decrements CTR, then branches if the decremented CTR M:63 is 0.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_after_a_bdz_of_a_ctr_that_holds_only_values_of_a_class_leaves_every_site_unresolved() {
    assert_eq!(toc_loop_sites(&[bc(0b10010, 4), bctr()]), EVERY_UNRESOLVED);
}

/// The `mfctr` copies a CTR that holds only values of the class into r6,
/// the `addi` adds 4 to the copy, and the second `mtctr` puts it back
/// into CTR.
///
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_stepped_mfctr_copy_of_a_ctr_of_a_class_leaves_every_site_unresolved() {
    assert_eq!(
        toc_loop_sites(&[mfctr(6), addi(6, 6, 4), mtctr(6), bctr()]),
        EVERY_UNRESOLVED
    );
}

/// Twenty paths bring r3 to the `addi`, which adds 8, so the `mtctr`
/// takes a lost value that keeps one progression. Each word at a multiple
/// of 8 is in its class, the window's `sc` among them. The refinement
/// first raises the bound of the join to keep all twenty constants, so
/// the scan needs no class.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_stepped_gpr_whose_join_the_refinement_raises_enters_no_word_of_a_class() {
    let mut words = writers(20, &[]);
    words.extend([addi(3, 3, 8), mtctr(3), bctr()]);
    assert_eq!(away_sites(&words), WINDOW_RESOLVED);
}

/// The loop subtracts 0x20 from r3 from 0x5000, and the sums wrap below
/// 0, so r3 holds only multiples of 0x20. The loop that subtracts 0x10
/// from 0x5008 gives each word at 8 past a multiple of 0x10, the window's
/// `sc` among them.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:15 s:1.12.2] Effective address arithmetic wraps from 2^64 - 1 to 0 in 64-bit mode, and from 2^32 - 1 to 0 in 32-bit mode.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_a_loop_steps_by_a_negative_constant_enters_only_the_words_of_its_class() {
    let mut words = stepped_loop(NOWHERE, -0x20);
    words.extend([mtctr(3), bctr()]);
    assert_eq!(away_sites(&words), WINDOW_RESOLVED);
    let mut words = vec![NOP, NOP];
    words.extend(stepped_loop(NOWHERE + 8, -0x10));
    words.extend([mtctr(3), bctr()]);
    assert_eq!(away_sites(&words), SC_ENTERED);
}

/// `addis rt,ra,imm`.
fn addis(rt: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Addis { rt, ra, imm })
}

/// The loop adds 0x1_0000 to r3 with `addis`, so r3 holds only values
/// with the low-order 16 bits of its first constant. From 0x5000 no word
/// has them. From 0x1_3008, which names no word, the window's `sc` has
/// them.
///
/// [PPC-Book1 p:51 s:3.3.8] `addis` adds EXTS(SI || 0x0000) to RA, or to 0 when RA is 0.
/// [PPC-Book1 p:66 s:3.3.11] `ori` ORs RS with UI and places the result into RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_a_loop_steps_with_addis_enters_only_the_words_of_its_low_half() {
    let tail = [addis(3, 3, 1), bc(0b01100, -4), mtctr(3), bctr()];
    let mut words = vec![addi(3, 0, NOWHERE.cast_signed())];
    words.extend(tail);
    assert_eq!(away_sites(&words), WINDOW_RESOLVED);
    let ori = word(PpuInstruction::Ori {
        ra: 3,
        rs: 3,
        imm: 0x3008,
    });
    let mut words = vec![addis(3, 0, 1), ori];
    words.extend(tail);
    assert_eq!(away_sites(&words), SC_ENTERED);
}

/// The `mtlr` copies the loop GPR into LR and the `mflr` copies LR into
/// r5, so r5 keeps the class of the loop. An `addi` of 8 after the `mflr`
/// adds a step that is not the step of the loop.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:82 s:3.3.13] `mflr`, a form of `mfspr`, copies LR into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_mflr_copy_of_a_loop_gpr_keeps_its_class_until_another_step() {
    let mut words = stepped_loop(NOWHERE, 0x20);
    words.extend([mtlr(3), mflr(5), mtctr(5), bctr()]);
    assert_eq!(away_sites(&words), WINDOW_RESOLVED);
    let mut words = stepped_loop(NOWHERE, 0x20);
    words.extend([mtlr(3), mflr(5), addi(5, 5, 8), mtctr(5), bctr()]);
    assert_eq!(away_sites(&words), EVERY_UNRESOLVED);
}

/// `count` writers that each build their own multiple of 0x20 from 0x5000
/// in r3 and copy it into LR, with a `blt` after each to the word after
/// the last `blt`. Each writer takes four words, so only the `lis` of
/// every second writer is at a multiple of 0x20.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
fn lr_writers_by_0x20(count: u16) -> Vec<u32> {
    let mut words = Vec::new();
    for writer in 0..count {
        words.extend(build(3, NOWHERE + 0x20 * writer));
        words.push(mtlr(3));
        let later = i16::try_from(4 * (count - 1 - writer)).expect("a short skip");
        words.push(skip(later));
    }
    words
}

/// Sixty-five writers bring more constants in LR than a raised bound
/// keeps, so the join at the `mflr` drops some of them. Each is a multiple
/// of 0x20, and the `addi` steps the copy by 0x20, so r5 holds only
/// multiples of 0x20. No word from the join to the `bctr` is one.
///
/// [PPC-Book1 p:82 s:3.3.13] `mflr`, a form of `mfspr`, copies LR into RT.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_stepped_mflr_copy_of_an_lr_past_the_raised_limit_enters_no_word_after_a_li_r11() {
    let mut words = lr_writers_by_0x20(RAISED_LIMIT + 1);
    words.extend([mflr(5), addi(5, 5, 0x20), mtctr(5), bctr()]);
    assert_eq!(away_sites(&words), WINDOW_RESOLVED);
}

/// The `bl` calls the routine after the `bctr`. The routine adds 4 to
/// CTR and returns, so the `bctr` takes a CTR off the class, such as the
/// window's `nop`.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_ctr_of_a_class_after_a_call_whose_routine_steps_ctr_leaves_every_site_unresolved() {
    assert_eq!(
        toc_loop_sites(&[bl(8), bctr(), mfctr(6), addi(6, 6, 4), mtctr(6), PPC_BLR]),
        EVERY_UNRESOLVED
    );
}

/// The `mtlr r3` copies the class of the loop into LR, and the `blt` takes
/// it to the join at word 9. The fall-through takes LR from the `mtlr r4`
/// at word 8, 0x20 past [`AWAY`], a word of the class. An indirect
/// branch can enter it with any value in r4, so LR at the join can hold
/// a value off the class.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_of_an_lr_class_joined_with_an_lr_from_a_word_of_the_class_leaves_every_site_unresolved() {
    let mut words = stepped_loop(NOWHERE, 0x20);
    words.extend([
        mtlr(3),
        addi(4, 0, 0x7000),
        bc(0b01100, 16),
        NOP,
        NOP,
        mtlr(4),
        NOP,
        PPC_BLR,
    ]);
    assert_eq!(away_sites(&words), EVERY_UNRESOLVED);
}

/// Seventeen loops each add 0x1000 to r3, and each loop join drops
/// constants of r3, more drops than a lost mark names. Each class of the
/// named drops holds only the words at a multiple of 0x1000, and each
/// such word is a `li`. The drop that the mark does not name can hold any
/// class.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_more_loops_step_than_a_mark_names_leaves_every_site_unresolved() {
    let mut words = vec![addi(3, 0, NOWHERE.cast_signed())];
    for _ in 0..17 {
        words.extend([addi(3, 3, 0x1000), bc(0b01100, -4)]);
    }
    words.extend([mtctr(3), bctr()]);
    assert_eq!(away_sites(&words), EVERY_UNRESOLVED);
}

/// The `blt` brings 0x4FF8 to the loop and the fall-through brings
/// 0x5008. The two share only their low-order 4 bits, so r3 holds values
/// at 8 past a multiple of 0x10, the window's `sc` among them.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0.
#[test]
fn a_bctr_of_a_loop_gpr_from_two_constants_enters_the_class_that_both_share() {
    let words = [
        addi(3, 0, 0x4FF8),
        bc(0b01100, 8),
        addi(3, 0, 0x5008),
        addi(3, 3, 0x20),
        bc(0b01100, -4),
        mtctr(3),
        bctr(),
    ];
    assert_eq!(away_sites(&words), SC_ENTERED);
}

/// The loop adds 2 to r3, so r3 shares fewer low-order bits than a class
/// needs, and its values can reach each word.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_that_a_loop_steps_by_2_leaves_every_site_unresolved() {
    let mut words = stepped_loop(NOWHERE, 2);
    words.extend([mtctr(3), bctr()]);
    assert_eq!(away_sites(&words), EVERY_UNRESOLVED);
}

/// The `bl` links to the next word, so it runs no routine and CTR keeps
/// only values of the class at the `bctr`.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
#[test]
fn a_bctr_of_a_ctr_of_a_class_after_a_bl_to_the_next_word_enters_no_word_after_a_li_r11() {
    assert_eq!(toc_loop_sites(&[bl(4), bctr()]), WINDOW_RESOLVED);
}
