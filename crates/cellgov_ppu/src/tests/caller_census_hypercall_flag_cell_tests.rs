use super::hypercall_constant_join_tests::{
    bctr, build, mtctr, sites, skip, twenty_pass_loop, EVERY_UNRESOLVED, NOWHERE, WINDOW_RESOLVED,
};
use super::hypercall_ctr_copy_tests::mfctr;
use super::hypercall_ctr_decrement_tests::{bc, quiet, window, VECTOR};
use super::hypercall_entry_tests::{code, ordinals, word};
use super::hypercall_tests::KERNEL;
use super::hypercall_unfollowed_write_tests::mfspr;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;
use cellgov_ps3_abi::hw::ppu_supervisor::SPR_SPRG0;

use PpuInstruction as I;

/// The kernel address of the routine that a caller at [`VECTOR`] calls.
const ROUTINE: u64 = KERNEL + 0x7000;

/// The loop that marks r3 lost, then `tail`, then `mtctr r4; bctr`.
fn loop_then_bctr_of_r4(tail: &[u32]) -> Vec<u32> {
    let mut words = twenty_pass_loop();
    words.extend_from_slice(tail);
    words.extend([mtctr(4), bctr()]);
    words
}

/// `cmpd crf,ra,ra`.
fn cmpd(bf: u8, ra: u8) -> u32 {
    word(I::Cmpd { bf, ra, rb: ra })
}

/// `addc r0,r3,r3`, with OE = `oe` and Rc = `rc`.
fn addc_of_r3(oe: bool, rc: bool) -> u32 {
    word(I::Addc {
        rt: 0,
        ra: 3,
        rb: 3,
        oe,
        rc,
    })
}

/// `add r0,r3,r3`, with OE = `oe` and Rc = `rc`.
fn add_of_r3(oe: bool, rc: bool) -> u32 {
    word(I::Add {
        rt: 0,
        ra: 3,
        rb: 3,
        oe,
        rc,
    })
}

/// `adde r4,r5,r5`.
fn adde_r4() -> u32 {
    word(I::Adde {
        rt: 4,
        ra: 5,
        rb: 5,
        oe: false,
        rc: false,
    })
}

/// Each word that moves CR, XER or an SPR into r4, by the cells that it
/// reads, and `adde r4,r5,r5`, which reads XER\[CA\].
///
/// [PPC-Book1 p:83 s:3.3.13] `mfcr` places CR into RT.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT, and `mfspr` of XER places XER into RT.
/// [PPC-Book1 p:54 s:3.3.8] `adde` places (RA) + (RB) + CA into RT.
fn readers() -> Readers {
    Readers {
        cr: word(I::Mfcr { rt: 4 }),
        xer: word(I::Mfxer { rt: 4 }),
        spr: mfspr(4, SPR_SPRG0),
        ca: adde_r4(),
    }
}

/// The words of [`readers`].
struct Readers {
    cr: u32,
    xer: u32,
    spr: u32,
    ca: u32,
}

/// [PPC-Book1 p:60 s:3.3.9] A compare sets the CR field BF from a comparison of RA with SI, UI or RB, and copies XER\[SO\] into bit 3 of the field.
/// [PPC-Book1 p:53 s:3.3.8] `addc` places (RA) + (RB) into RT and sets CA.
/// [PPC-Book1 p:18 s:2.3.1] A fixed-point instruction with Rc = 1 sets CR0 from a compare of the result with 0 and from XER\[SO\].
/// [PPC-Book1 p:32 s:3.2.2] An XO-form add with OE = 1 sets OV, and an instruction that sets OV sets SO.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_flag_write_of_a_lost_gpr_marks_only_the_cells_that_it_writes() {
    let r = readers();
    let lost = [
        (cmpd(7, 3), r.cr),
        (addc_of_r3(false, false), r.ca),
        (addc_of_r3(false, false), r.xer),
        (add_of_r3(false, true), r.cr),
        (add_of_r3(true, false), r.xer),
    ];
    for (writer, reader) in lost {
        let words = loop_then_bctr_of_r4(&[writer, reader]);
        assert_eq!(
            sites(VECTOR, &words),
            EVERY_UNRESOLVED,
            "{writer:08x} {reader:08x}"
        );
    }
    let kept = [
        (cmpd(7, 3), r.xer),
        (cmpd(7, 3), r.spr),
        (cmpd(7, 3), r.ca),
        (addc_of_r3(false, false), r.cr),
        (addc_of_r3(false, false), r.spr),
        (add_of_r3(false, true), r.xer),
        (add_of_r3(false, true), r.ca),
        (add_of_r3(true, false), r.cr),
        (add_of_r3(true, false), r.ca),
    ];
    for (writer, reader) in kept {
        let words = loop_then_bctr_of_r4(&[writer, reader]);
        assert_eq!(
            sites(VECTOR, &words),
            WINDOW_RESOLVED,
            "{writer:08x} {reader:08x}"
        );
    }
}

/// `addo` puts the lost r3 into XER\[SO\], and the compare of r5 copies
/// SO into cr7.
///
/// [PPC-Book1 p:32 s:3.2.2] An XO-form add with OE = 1 sets OV, and an instruction that sets OV sets SO.
/// [PPC-Book1 p:60 s:3.3.9] A compare sets the CR field BF from a comparison of RA with SI, UI or RB, and copies XER\[SO\] into bit 3 of the field.
/// [PPC-Book1 p:83 s:3.3.13] `mfcr` places CR into RT.
#[test]
fn a_bctr_of_cr_after_a_compare_copies_a_lost_summary_overflow_leaves_every_site_unresolved() {
    let mfcr = word(I::Mfcr { rt: 4 });
    let words = loop_then_bctr_of_r4(&[add_of_r3(true, false), cmpd(7, 5), mfcr]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
    let words = loop_then_bctr_of_r4(&[add_of_r3(false, false), cmpd(7, 5), mfcr]);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// A later write of each bit of cr7, or of XER\[CA\], from r5 replaces
/// the mark that the lost r3 put there. `mtocrf` with two bits of FXM
/// set writes CR in a way that the documents do not define, so cr7 keeps
/// its mark. The other bits of XER share one cell with the SPRs that the
/// scan does not follow, so `mtxer` writes a part of that cell, and the
/// mark of `mtxer r3` stays there.
///
/// [PPC-Book1 p:60 s:3.3.9] A compare sets the CR field BF from a comparison of RA with SI, UI or RB, and copies XER\[SO\] into bit 3 of the field.
/// [PPC-Book1 p:83 s:3.3.13] When bit i of FXM is 1, `mtcrf` sets CR field i to the corresponding field of RS 32:63.
/// [PPC-Book1 p:124 s:5.1.1] When FXM has exactly one bit set, `mtocrf` places RS into CR field n; otherwise the contents of CR are undefined.
/// [PPC-Book1 p:32 s:3.2.2] An `mtspr` to XER sets SO and OV from the values that it supplies.
/// [PPC-Book1 p:51 s:3.3.8] `addic` always sets CA from the carry of the result.
#[test]
fn a_write_of_each_bit_of_a_cell_from_an_unmarked_gpr_clears_its_mark() {
    let r = readers();
    let addic = word(I::Addic {
        rt: 0,
        ra: 5,
        imm: 1,
    });
    let cleared = [
        [cmpd(7, 3), cmpd(7, 5), r.cr],
        [cmpd(7, 3), word(I::Mtcrf { rs: 5, crm: 0x01 }), r.cr],
        [cmpd(7, 3), word(I::Mtocrf { rs: 5, crm: 0x01 }), r.cr],
        [addc_of_r3(false, false), addic, r.ca],
        [addc_of_r3(false, false), word(I::Mtxer { rs: 5 }), r.ca],
    ];
    for words in cleared {
        assert_eq!(
            sites(VECTOR, &loop_then_bctr_of_r4(&words)),
            WINDOW_RESOLVED,
            "{words:08x?}"
        );
    }
    let kept = [
        [cmpd(7, 3), word(I::Mtcrf { rs: 5, crm: 0x80 }), r.cr],
        [cmpd(7, 3), word(I::Mtocrf { rs: 5, crm: 0x03 }), r.cr],
        [word(I::Mtxer { rs: 3 }), word(I::Mtxer { rs: 5 }), r.xer],
    ];
    for words in kept {
        assert_eq!(
            sites(VECTOR, &loop_then_bctr_of_r4(&words)),
            EVERY_UNRESOLVED,
            "{words:08x?}"
        );
    }
}

/// The PPE places the whole CR into RT for each FXM of `mfocrf`, so an
/// `mfocrf` of cr0 also takes the mark of cr7.
///
/// [PPC-Book1 p:60 s:3.3.9] A compare sets the CR field BF from a comparison of RA with SI, UI or RB, and copies XER\[SO\] into bit 3 of the field.
/// [PPC-Book1 p:124 s:5.1.1] `mtocrf` and `mfocrf` move one CR field between CR and a GPR.
#[test]
fn a_bctr_of_an_mfocrf_of_cr0_after_a_compare_of_a_lost_gpr_into_cr7_leaves_every_site_unresolved()
{
    let mfocrf = word(I::Mfocrf { rt: 4, crm: 0x80 });
    let words = loop_then_bctr_of_r4(&[cmpd(7, 3), mfocrf]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// `mcrf` and `cror` copy the mark of cr7 into cr0 before the compare of
/// r5 replaces the mark of cr7.
///
/// [PPC-Book1 p:30 s:2.4.4] `mcrf` copies CR field BFA into CR field BF.
/// [PPC-Book1 p:28 s:2.4.3] `cror` places the OR of the CR bits BA and BB into the CR bit BT.
/// [PPC-Book1 p:135 s:6.1] `mcrxr` copies XER 32:35 into CR field BF.
/// [PPC-Book1 p:83 s:3.3.13] `mfcr` places CR into RT.
#[test]
fn a_bctr_of_cr_after_a_cr_field_move_of_a_lost_mark_leaves_every_site_unresolved() {
    let mfcr = word(I::Mfcr { rt: 4 });
    let moves = [
        [cmpd(7, 3), word(I::Mcrf { crfd: 0, crfs: 7 })],
        [
            cmpd(7, 3),
            word(I::Cror {
                bt: 2,
                ba: 30,
                bb: 30,
            }),
        ],
        [addc_of_r3(false, false), word(I::Mcrxr { bf: 0 })],
    ];
    for [writer, moved] in moves {
        let words = loop_then_bctr_of_r4(&[writer, moved, cmpd(7, 5), mfcr]);
        assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED, "{moved:08x}");
    }
}

/// The `blt` skips the compare of r5 on its taken path, so the join word
/// gets the mark of cr7 from that path and no mark in the other cells.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:60 s:3.3.9] A compare sets the CR field BF from a comparison of RA with SI, UI or RB, and copies XER\[SO\] into bit 3 of the field.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_join_word_takes_the_mark_of_each_cell_from_the_path_that_brings_it() {
    let r = readers();
    let joined = |reader: u32| loop_then_bctr_of_r4(&[cmpd(7, 3), skip(1), cmpd(7, 5), reader]);
    assert_eq!(sites(VECTOR, &joined(r.cr)), EVERY_UNRESOLVED);
    for reader in [r.xer, r.spr, r.ca] {
        assert_eq!(
            sites(VECTOR, &joined(reader)),
            WINDOW_RESOLVED,
            "{reader:08x}"
        );
    }
}

/// `lis r3,0; ori r3,r3,0x5000; mtctr r3; mfctr r4`, then `tail`. r4
/// holds a copy of the constant 0x5000.
///
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
fn known_copy_in_r4(tail: &[u32]) -> Vec<u32> {
    let mut words = build(3, NOWHERE).to_vec();
    words.extend([mtctr(3), mfctr(4)]);
    words.extend_from_slice(tail);
    words
}

/// A compare of the copy in r4 sets the CTR flow's cell of cr7. A move
/// out of another cell gives no copy, and a compare of r6 into cr7 clears
/// the cell, unless a join brings it from a path that skips that compare.
///
/// [PPC-Book1 p:60 s:3.3.9] A compare sets the CR field BF from a comparison of RA with SI, UI or RB, and copies XER\[SO\] into bit 3 of the field.
/// [PPC-Book1 p:83 s:3.3.13] `mfcr` places CR into RT.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT, and `mfspr` of XER places XER into RT.
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_compare_of_a_ctr_copy_sets_only_the_ctr_flow_cell_of_its_field() {
    let mfcr = word(I::Mfcr { rt: 5 });
    let tail = [mtctr(5), bctr()];
    let with = |words: &[u32]| known_copy_in_r4(&[words, &tail].concat());
    assert_eq!(sites(VECTOR, &with(&[cmpd(7, 4), mfcr])), EVERY_UNRESOLVED);
    let joined = with(&[cmpd(7, 4), skip(1), cmpd(7, 6), mfcr]);
    assert_eq!(sites(VECTOR, &joined), EVERY_UNRESOLVED);
    for words in [
        with(&[cmpd(7, 4), word(I::Mfxer { rt: 5 })]),
        with(&[cmpd(7, 4), mfspr(5, SPR_SPRG0)]),
        with(&[cmpd(7, 4), cmpd(7, 6), mfcr]),
    ] {
        assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED, "{words:08x?}");
    }
}

/// `b` with the displacement `offset`.
fn b(offset: i32) -> u32 {
    word(I::B {
        offset,
        aa: false,
        link: false,
    })
}

/// The ordinals when a caller at [`VECTOR`] runs `bl ROUTINE`, then
/// `after`, and the routine runs a `bdnz $+4` over a CTR of 0x5000, then
/// `mfctr r6`, `cmpd cr7,r6,r6` and `blr`.
fn after_a_routine_that_compares_a_lost_copy(after: &[u32]) -> Vec<(u64, Option<u16>)> {
    after_a_call_into(&[cmpd(7, 6), PPC_BLR], after)
}

/// The ordinals when a caller at [`VECTOR`] runs `bl ROUTINE`, then
/// `after`, and the routine runs a `bdnz $+4` over a CTR of 0x5000, then
/// `mfctr r6` and `tail`. r6 then holds a copy that the scan cannot prove.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
fn after_a_call_into(tail: &[u32], after: &[u32]) -> Vec<(u64, Option<u16>)> {
    let offset = i32::try_from(ROUTINE.wrapping_sub(VECTOR).cast_signed()).expect("in reach");
    let mut caller = vec![word(I::B {
        offset,
        aa: false,
        link: true,
    })];
    caller.extend_from_slice(after);
    let mut routine = build(3, NOWHERE).to_vec();
    routine.extend([mtctr(3), bc(0b10000, 4), mfctr(6)]);
    routine.extend_from_slice(tail);
    let segments = [
        code(VECTOR, &caller),
        window(&[]),
        quiet(),
        code(ROUTINE, &routine),
    ];
    ordinals(0, &segments)
}

/// The return brings the cell of cr7 to the word after the call, and no
/// other cell.
///
/// [PPC-Book1 p:83 s:3.3.13] `mfcr` places CR into RT.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT, and `mfspr` of XER places XER into RT.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_return_brings_only_the_ctr_flow_cells_that_its_routine_set() {
    let tail = |reader: u32| [reader, mtctr(5), bctr()];
    let mfcr = word(I::Mfcr { rt: 5 });
    assert_eq!(
        after_a_routine_that_compares_a_lost_copy(&tail(mfcr)),
        EVERY_UNRESOLVED
    );
    for reader in [word(I::Mfxer { rt: 5 }), mfspr(5, SPR_SPRG0)] {
        assert_eq!(
            after_a_routine_that_compares_a_lost_copy(&tail(reader)),
            WINDOW_RESOLVED,
            "{reader:08x}"
        );
    }
}

/// The `blt` skips the compare, so the `blr` first returns with r6 and no
/// cell. The `b` brings the cell of cr7 back to the `blr`, which the flow
/// has stepped, with each other register and slot unchanged. The word
/// after the call gets the cell only when that second return counts as a
/// change.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:24 s:2.4] `b` reaches CIA + EXTS(displacement).
/// [PPC-Book1 p:60 s:3.3.9] A compare sets the CR field BF from a comparison of RA with SI, UI or RB, and copies XER\[SO\] into bit 3 of the field.
/// [PPC-Book1 p:83 s:3.3.13] `mfcr` places CR into RT.
#[test]
fn a_second_return_that_brings_only_a_new_ctr_flow_cell_reaches_the_word_after_the_call() {
    let tail = [skip(1), PPC_BLR, cmpd(7, 6), b(-8)];
    let after = [word(I::Mfcr { rt: 5 }), mtctr(5), bctr()];
    assert_eq!(after_a_call_into(&tail, &after), EVERY_UNRESOLVED);
}

/// The first `blt` skips to the compare of the lost r3, whose `b` brings
/// the mark of cr7 back to the second `blt`, which the flow has walked,
/// with each other register unchanged. The `mfcr` at the next join word
/// gets the mark only when that join counts as a change.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:24 s:2.4] `b` reaches CIA + EXTS(displacement).
/// [PPC-Book1 p:60 s:3.3.9] A compare sets the CR field BF from a comparison of RA with SI, UI or RB, and copies XER\[SO\] into bit 3 of the field.
/// [PPC-Book1 p:83 s:3.3.13] `mfcr` places CR into RT.
#[test]
fn a_mark_that_a_later_path_brings_to_a_walked_join_word_reaches_the_next_join_word() {
    let mut words = twenty_pass_loop();
    words.extend([
        skip(4),
        skip(0),
        word(I::Mfcr { rt: 4 }),
        mtctr(4),
        bctr(),
        cmpd(7, 3),
        b(-20),
    ]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}
