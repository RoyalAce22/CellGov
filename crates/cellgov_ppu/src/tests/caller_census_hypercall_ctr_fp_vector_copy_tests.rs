use super::hypercall_constant_join_tests::{build, NOWHERE};
use super::hypercall_ctr_copy_tests::{after_bdnz_falls_through, call, copy_stub, mfctr, mtctr};
use super::hypercall_ctr_decrement_tests::{bc, quiet, window, QUIET_SC, VECTOR};
use super::hypercall_ctr_stack_copy_tests::{RESOLVED, UNRESOLVED};
use super::hypercall_ctr_virtual_tests::{bctr, sites};
use super::hypercall_entry_tests::{code, ordinals, word};
use super::hypercall_tests::{li, KERNEL};
use super::*;
use crate::instruction::ops::{Fp59Op, Fp63Op, VxOp};
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The kernel address of the routine that a caller at [`VECTOR`] calls.
const ROUTINE: u64 = KERNEL + 0x7000;

/// `std rs,imm(r1)`.
fn std(rs: u8, imm: i16) -> u32 {
    word(PpuInstruction::Std { rs, ra: 1, imm })
}

/// `ld rt,imm(r1)`.
fn ld(rt: u8, imm: i16) -> u32 {
    word(PpuInstruction::Ld { rt, ra: 1, imm })
}

/// `lwz rt,imm(r1)`.
fn lwz(rt: u8, imm: i16) -> u32 {
    word(PpuInstruction::Lwz { rt, ra: 1, imm })
}

/// `lfd frt,imm(r1)`.
fn lfd(frt: u8, imm: i16) -> u32 {
    word(PpuInstruction::Lfd { frt, ra: 1, imm })
}

/// `stfd frs,imm(r1)`.
fn stfd(frs: u8, imm: i16) -> u32 {
    word(PpuInstruction::Stfd { frs, ra: 1, imm })
}

/// `lvx vt,0,r1`.
fn lvx_r1(vt: u8) -> u32 {
    word(PpuInstruction::Lvx { vt, ra: 0, rb: 1 })
}

/// `stvx vs,0,r1`.
fn stvx_r1(vs: u8) -> u32 {
    word(PpuInstruction::Stvx { vs, ra: 0, rb: 1 })
}

fn mfcr(rt: u8) -> u32 {
    word(PpuInstruction::Mfcr { rt })
}

/// The sites when a stub copies a CTR that a call leaves at any value
/// into r0, runs `between`, and moves r0 into CTR.
fn after_a_call_copies_ctr(between: &[u32]) -> Vec<(u64, Option<u16>)> {
    let mut words = vec![call(), mfctr(0)];
    words.extend_from_slice(between);
    words.push(mtctr(0));
    sites(vec![copy_stub(&words)])
}

/// `lis r3,0; ori r3,r3,0x5000; mtctr r3; bdnz $+4; mfctr r4;
/// std r4,16(r1)`. The slot at 16(r1) then holds a copy of a CTR that the
/// scan cannot prove.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
fn lost_copy_at_16() -> Vec<u32> {
    let mut words = build(3, NOWHERE).to_vec();
    words.extend([mtctr(3), bc(0b10000, 4), mfctr(4), std(4, 16)]);
    words
}

/// [`lost_copy_at_16`], then `lfd f1,16(r1)` when `load` is set, then
/// `li r4,0; std r4,16(r1)`. Only f1 can then hold the copy.
fn lost_copy_only_in_f1(load: bool) -> Vec<u32> {
    let mut words = lost_copy_at_16();
    if load {
        words.push(lfd(1, 16));
    }
    words.extend([li(4, 0), std(4, 16)]);
    words
}

/// `stfd f1,24(r1); ld r5,24(r1); mtctr r5; bctr`.
fn bctr_of_f1() -> [u32; 4] {
    [stfd(1, 24), ld(5, 24), mtctr(5), bctr()]
}

/// The ordinals when a caller at [`VECTOR`] runs `before`, `bl ROUTINE`
/// and `after`, and the routine at [`ROUTINE`] runs `body`.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
fn with_routine(before: &[u32], after: &[u32], body: &[u32]) -> Vec<(u64, Option<u16>)> {
    let mut caller = before.to_vec();
    let at = VECTOR + 4 * u64::try_from(caller.len()).expect("in reach");
    let offset = i32::try_from(ROUTINE.wrapping_sub(at).cast_signed()).expect("in reach");
    caller.push(word(PpuInstruction::B {
        offset,
        aa: false,
        link: true,
    }));
    caller.extend_from_slice(after);
    let segments = [
        code(VECTOR, &caller),
        window(&[]),
        quiet(),
        code(ROUTINE, body),
    ];
    ordinals(0, &segments)
}

/// [PPC-Book1 p:105 s:4.6.2] `lfd` loads the doubleword at EA into FRT.
/// [PPC-Book1 p:108 s:4.6.3] `stfd` stores FRS into the doubleword at EA.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
fn a_copy_of_ctr_moved_through_an_fpr_by_lfd_and_stfd_leaves_every_site_unresolved() {
    let between = [std(0, 16), lfd(1, 16), stfd(1, 24), ld(0, 24)];
    assert_eq!(after_a_call_copies_ctr(&between), UNRESOLVED);
}

/// [PPC-Book1 p:104 s:4.6.2] `lfs` loads the word at EA, converted to double format, into FRT.
/// [PPC-Book1 p:107 s:4.6.3] `stfs` stores FRS, converted to single format, into the word at EA.
/// [PPC-Book1 p:37 s:3.3.2] `lwz` loads the word at EA into RT 32:63 and sets RT 0:31 to 0.
#[test]
fn a_copy_of_ctr_moved_through_an_fpr_by_lfs_and_stfs_leaves_every_site_unresolved() {
    let lfs = word(PpuInstruction::Lfs {
        frt: 1,
        ra: 1,
        imm: 20,
    });
    let stfs = word(PpuInstruction::Stfs {
        frs: 1,
        ra: 1,
        imm: 28,
    });
    let between = [std(0, 16), lfs, stfs, lwz(0, 28)];
    assert_eq!(after_a_call_copies_ctr(&between), UNRESOLVED);
}

/// No floating-point load reads the copy, so the `stfd` writes a value
/// that holds no copy.
///
/// [PPC-Book1 p:108 s:4.6.3] `stfd` stores FRS into the doubleword at EA.
#[test]
fn an_fpr_store_with_no_copy_loaded_into_an_fpr_keeps_each_site_resolved() {
    let between = [std(0, 16), stfd(1, 24), ld(0, 24)];
    assert_eq!(after_a_call_copies_ctr(&between), RESOLVED);
}

/// The `li` and the second `std` remove the copy from r0 and from 0(r1),
/// so only v0 holds it at the `stvx`. Without the `lvx`, v0 holds no
/// copy.
///
/// [AltiVec-PEM p:6-21 s:6.2] `lvx` loads the quadword at EA = ((RA|0) + (RB)) & ~0xF.
/// [AltiVec-PEM p:6-28 s:6.2] `stvx` stores VS into the quadword at EA = ((RA|0) + (RB)) & ~0xF.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
fn a_copy_of_ctr_moved_through_a_vr_by_lvx_and_stvx_leaves_every_site_unresolved() {
    let through = |load: bool| {
        let mut between = vec![std(0, 0)];
        if load {
            between.push(lvx_r1(0));
        }
        between.extend([li(0, 0), std(0, 0), stvx_r1(0), ld(0, 0)]);
        after_a_call_copies_ctr(&between)
    };
    assert_eq!(through(true), UNRESOLVED);
    assert_eq!(through(false), RESOLVED);
}

/// The `bdnz` leaves a CTR of 0. The `stvx` can write a byte of the copy
/// at 0(r1), so the `ld` then loads a value that the scan cannot prove.
///
/// [AltiVec-PEM p:6-28 s:6.2] `stvx` stores VS into the quadword at EA = ((RA|0) + (RB)) & ~0xF.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
fn a_vector_store_at_r1_over_a_known_ctr_copy_leaves_a_value_the_scan_cannot_prove() {
    assert_eq!(
        after_bdnz_falls_through(&[mfctr(0), std(0, 0), stvx_r1(0), ld(0, 0), mtctr(0)]),
        [(4, None), (QUIET_SC, None)]
    );
    assert_eq!(
        after_bdnz_falls_through(&[mfctr(0), std(0, 0), ld(0, 0), mtctr(0)]),
        [(4, None), (QUIET_SC, Some(77))]
    );
}

/// The `bdnz` leaves a CTR of 0, and the slot at 16(r1) holds a copy of
/// it. An `lfd` of the doubleword just below or just above that slot
/// reads no byte of the copy, so the `stfd` writes a value that holds no
/// copy over the slot, and the `bctr` reaches no address that a copy of
/// CTR names.
///
/// [PPC-Book1 p:105 s:4.6.2] `lfd` loads the doubleword at EA into FRT.
/// [PPC-Book1 p:108 s:4.6.3] `stfd` stores FRS into the doubleword at EA.
#[test]
fn an_fpr_load_next_to_a_known_ctr_copy_reads_no_copy() {
    for (load, slot) in [(24, 16), (8, 16)] {
        let between = [
            mfctr(0),
            std(0, slot),
            lfd(1, load),
            stfd(1, slot),
            ld(0, slot),
            mtctr(0),
        ];
        assert_eq!(
            after_bdnz_falls_through(&between),
            [(4, Some(75)), (QUIET_SC, Some(77))],
            "lfd at {load}"
        );
    }
}

/// The `bdnz` leaves a CTR of 0. A `stvx` at r1 writes the 16 bytes at
/// r1 rounded down to a multiple of 16, so it can write each byte from
/// -15(r1) to 15(r1). The copy at 16(r1) lies above every such byte and
/// keeps its constant. The copy at -8(r1) can lose a byte, so the `ld`
/// loads a value that the scan cannot prove.
///
/// [AltiVec-PEM p:6-28 s:6.2] `stvx` stores VS into the quadword at EA = ((RA|0) + (RB)) & ~0xF.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
fn a_vector_store_at_r1_loses_only_a_known_ctr_copy_that_it_can_write() {
    let through = |slot: i16| {
        after_bdnz_falls_through(&[mfctr(0), std(0, slot), stvx_r1(0), ld(0, slot), mtctr(0)])
    };
    assert_eq!(through(16), [(4, None), (QUIET_SC, Some(77))]);
    assert_eq!(through(-8), [(4, None), (QUIET_SC, None)]);
}

/// The `bdnz` leaves a CTR of 0. The mark of the FPRs keeps no constant,
/// so the copy that comes back from f1 is one that the scan cannot prove.
///
/// [PPC-Book1 p:105 s:4.6.2] `lfd` loads the doubleword at EA into FRT.
/// [PPC-Book1 p:108 s:4.6.3] `stfd` stores FRS into the doubleword at EA.
#[test]
fn a_known_ctr_copy_moved_through_an_fpr_becomes_one_the_scan_cannot_prove() {
    let between = [
        mfctr(0),
        std(0, 16),
        lfd(1, 16),
        stfd(1, 24),
        ld(0, 24),
        mtctr(0),
    ];
    assert_eq!(
        after_bdnz_falls_through(&between),
        [(4, None), (QUIET_SC, None)]
    );
}

/// The `stfd` is a join word: the first path reaches it before the `lfd`
/// runs, and the loop brings the mark back to it.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:108 s:4.6.3] `stfd` stores FRS into the doubleword at EA.
#[test]
fn a_copy_of_ctr_that_a_loop_brings_back_in_an_fpr_leaves_every_site_unresolved() {
    let between = [
        std(0, 16),
        stfd(1, 24),
        ld(0, 24),
        lfd(1, 16),
        bc(0b01100, -12),
    ];
    assert_eq!(after_a_call_copies_ctr(&between), UNRESOLVED);
}

/// The routine loads a copy of its lost CTR into f1 and returns, and the
/// caller stores f1 and moves it into CTR.
///
/// [PPC-Book1 p:105 s:4.6.2] `lfd` loads the doubleword at EA into FRT.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_fpr_that_a_routine_loaded_from_a_lost_ctr_copy_leaves_every_site_unresolved() {
    let body = |load: bool| {
        let mut words = lost_copy_only_in_f1(load);
        words.push(PPC_BLR);
        words
    };
    assert_eq!(with_routine(&[], &bctr_of_f1(), &body(true)), UNRESOLVED);
    assert_eq!(with_routine(&[], &bctr_of_f1(), &body(false)), RESOLVED);
}

/// The caller loads a copy of its lost CTR into f1 before the call, and
/// the routine stores f1 and moves it into CTR.
///
/// [PPC-Book1 p:105 s:4.6.2] `lfd` loads the doubleword at EA into FRT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_fpr_in_a_routine_that_the_caller_loaded_from_a_lost_ctr_copy_leaves_every_site_unresolved(
) {
    let called = |load: bool| with_routine(&lost_copy_only_in_f1(load), &[PPC_BLR], &bctr_of_f1());
    assert_eq!(called(true), UNRESOLVED);
    assert_eq!(called(false), RESOLVED);
}

/// `lvsl` computes v0 from the low-order bits of RB. r4 holds a copy of
/// CTR, and r9 holds none.
///
/// [AltiVec-PEM p:6-18 s:6.2] `lvsl` places a vector computed from sh = EA\[28:31\] into VD.
/// [AltiVec-PEM p:6-28 s:6.2] `stvx` stores VS into the quadword at EA = ((RA|0) + (RB)) & ~0xF.
#[test]
fn a_copy_of_ctr_moved_into_a_vr_by_lvsl_leaves_every_site_unresolved() {
    let lvsl = |rb: u8| word(PpuInstruction::Lvsl { vt: 0, ra: 0, rb });
    let through = |rb: u8| after_a_call_copies_ctr(&[mfctr(4), lvsl(rb), stvx_r1(0), ld(0, 0)]);
    assert_eq!(through(4), UNRESOLVED);
    assert_eq!(through(9), RESOLVED);
    let lvsl_r4_r4 = word(PpuInstruction::Lvsl {
        vt: 0,
        ra: 4,
        rb: 4,
    });
    let same = after_a_call_copies_ctr(&[mfctr(4), lvsl_r4_r4, stvx_r1(0), ld(0, 0)]);
    assert_eq!(same, UNRESOLVED);
}

/// Each word that sets a CR field from the FPRs, the VRs or FPSCR.
///
/// [PPC-Book1 p:119 s:4.6.7] `fcmpu` and `fcmpo` place the result of the compare into CR field BF.
/// [PPC-Book1 p:120 s:4.6.8] `mcrfs` copies FPSCR field BFA into CR field BF.
/// [PPC-Book1 p:18 s:2.3.1] A floating-point instruction can set CR field 1 as an implicit result.
/// [AltiVec-PEM p:2-7 s:2.4] A vector compare with Rc = 1 updates CR6.
fn cr_setters() -> Vec<u32> {
    use PpuInstruction as I;
    let fp63 = |op, frt, rc| {
        word(I::Fp63 {
            op,
            frt,
            fra: 1,
            frb: 2,
            frc: 0,
            rc,
        })
    };
    vec![
        fp63(Fp63Op::Fcmpu, 1 << 2, false),
        fp63(Fp63Op::Fcmpo, 7 << 2, false),
        word(I::Fp63 {
            op: Fp63Op::Mcrfs,
            frt: 2 << 2,
            fra: 0,
            frb: 0,
            frc: 0,
            rc: false,
        }),
        fp63(Fp63Op::Fadd, 3, true),
        word(I::Fp59 {
            op: Fp59Op::Fadds,
            frt: 3,
            fra: 1,
            frb: 2,
            frc: 0,
            rc: true,
        }),
        word(I::Vx {
            op: VxOp::Vcmpequb,
            rc: true,
            vt: 1,
            va: 0,
            vb: 2,
        }),
    ]
}

/// [PPC-Book1 p:83 s:3.3.13] `mfcr` places CR into RT 32:63.
/// [PPC-Book1 p:81 s:3.3.13] `mtctr`, a form of `mtspr`, copies RS into CTR.
#[test]
fn a_bctr_of_a_cr_field_set_while_an_fpr_holds_a_ctr_copy_leaves_every_site_unresolved() {
    let setters = cr_setters();
    assert_eq!(setters.len(), 6);
    for setter in setters {
        let between = [std(0, 16), lfd(1, 16), setter, mfcr(0)];
        assert_eq!(
            after_a_call_copies_ctr(&between),
            UNRESOLVED,
            "{setter:08x}"
        );
    }
    let unloaded = [std(0, 16), cr_setters()[0], mfcr(0)];
    assert_eq!(after_a_call_copies_ctr(&unloaded), RESOLVED);
}

/// An indexed floating-point load with RA at 0 and RB at 1 reads the
/// copy at 0(r1). The `li` and the second `std` then remove the copy
/// from r0 and from 0(r1), so only f1 holds it at the `stfd` or `stfs`.
///
/// [PPC-Book1 p:105 s:4.6.2] `lfdx` loads the doubleword at EA = (RA|0) + (RB) into FRT.
/// [PPC-Book1 p:104 s:4.6.2] `lfsx` loads the word at EA = (RA|0) + (RB) into FRT.
/// [PPC-Book1 p:108 s:4.6.3] `stfd` stores FRS into the doubleword at EA.
#[test]
fn a_copy_of_ctr_loaded_into_an_fpr_by_an_indexed_load_at_r1_leaves_every_site_unresolved() {
    let lfdx = word(PpuInstruction::Lfdx {
        frt: 1,
        ra: 0,
        rb: 1,
    });
    let lfsx = word(PpuInstruction::Lfsx {
        frt: 1,
        ra: 0,
        rb: 1,
    });
    let stfs = word(PpuInstruction::Stfs {
        frs: 1,
        ra: 1,
        imm: 28,
    });
    for (load, store, back) in [(lfdx, stfd(1, 24), ld(0, 24)), (lfsx, stfs, lwz(0, 28))] {
        let between = [std(0, 0), load, li(0, 0), std(0, 0), store, back];
        assert_eq!(after_a_call_copies_ctr(&between), UNRESOLVED, "{load:08x}");
    }
}

/// An indexed floating-point store with RA at 0 and RB at 1 writes f1,
/// which holds the copy, at 0(r1). The `li` and the second `std` remove
/// the copy from r0 and from 16(r1) first.
///
/// [PPC-Book1 p:108 s:4.6.3] `stfdx` stores FRS into the doubleword at EA = (RA|0) + (RB).
/// [PPC-Book1 p:107 s:4.6.3] `stfsx` stores FRS in single format into the word at EA = (RA|0) + (RB).
/// [PPC-Book1 p:109 s:4.6.3] `stfiwx` stores FRS 32:63 into the word at EA = (RA|0) + (RB).
#[test]
fn an_fpr_that_holds_a_ctr_copy_stored_by_an_indexed_store_at_r1_leaves_every_site_unresolved() {
    let stfdx = word(PpuInstruction::Stfdx {
        frs: 1,
        ra: 0,
        rb: 1,
    });
    let stfsx = word(PpuInstruction::Stfsx {
        frs: 1,
        ra: 0,
        rb: 1,
    });
    let stfiwx = word(PpuInstruction::Stfiwx {
        frs: 1,
        ra: 0,
        rb: 1,
    });
    for (store, back) in [(stfdx, ld(0, 0)), (stfsx, lwz(0, 0)), (stfiwx, lwz(0, 0))] {
        let between = [std(0, 16), lfd(1, 16), li(0, 0), std(0, 16), store, back];
        assert_eq!(after_a_call_copies_ctr(&between), UNRESOLVED, "{store:08x}");
    }
}
