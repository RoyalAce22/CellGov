use super::hypercall_constant_join_tests::{
    bctr as join_bctr, lr_writers, mflr, mtctr as join_mtctr, sites as join_sites,
    EVERY_UNRESOLVED, WINDOW_RESOLVED,
};
use super::hypercall_ctr_copy_tests::{call, copy_stub, mfctr, mtctr};
use super::hypercall_ctr_decrement_tests::{QUIET_SC, SC, VECTOR};
use super::hypercall_ctr_virtual_tests::{build, sites};
use super::hypercall_entry_tests::word;
use super::hypercall_join_drop_tests::RAISED_LIMIT;
use super::*;

fn mask(gprs: &[u8]) -> u32 {
    gprs.iter().fold(0, |mask, &gpr| mask | 1 << gpr)
}

fn decoded(instruction: PpuInstruction) -> PpuInstruction {
    crate::decode::decode(word(instruction)).expect("the word decodes")
}

fn lwzu(rt: u8, ra: u8) -> PpuInstruction {
    PpuInstruction::Lwzu { rt, ra, imm: 0 }
}

fn lmw(rt: u8, ra: u8) -> PpuInstruction {
    PpuInstruction::Lmw { rt, ra, imm: 0 }
}

fn lswi(rt: u8, ra: u8, nb: u8) -> PpuInstruction {
    PpuInstruction::Lswi { rt, ra, nb }
}

const UNRESOLVED: [(u64, Option<u16>); 2] = [(SC, None), (QUIET_SC, None)];

const RESOLVED: [(u64, Option<u16>); 2] = [(SC, Some(75)), (QUIET_SC, Some(77))];

/// [CBE-Handbook p:254 s:9.5.9] The PPE takes the illegal-instruction interrupt for a load or store with update whose RA is 0 or RT, and for a floating-point one whose RA is 0.
#[test]
fn an_update_form_that_the_ppe_refuses_surely_writes_no_gpr() {
    let refused = [
        lwzu(3, 3),
        lwzu(3, 0),
        PpuInstruction::Ldux {
            rt: 5,
            ra: 5,
            rb: 6,
        },
        PpuInstruction::Stwu {
            rs: 3,
            ra: 0,
            imm: 8,
        },
        PpuInstruction::Lfdu {
            frt: 1,
            ra: 0,
            imm: 8,
        },
    ];
    for instruction in refused {
        let decoded = decoded(instruction);
        assert_eq!(decoded, instruction);
        assert_ne!(written_gprs(&decoded), 0, "{instruction:?}");
        assert_eq!(surely_written_gprs(&decoded), 0, "{instruction:?}");
    }
}

/// [PPC-Book1 p:33 s:3.3.2] A load with update whose RA is neither 0 nor RT places EA into RA and loads RT.
#[test]
fn a_valid_load_with_update_surely_writes_rt_and_ra() {
    assert_eq!(surely_written_gprs(&decoded(lwzu(3, 4))), mask(&[3, 4]));
}

/// [PPC-Book1 p:104 s:4.6.2] `lfsu` is an invalid form only when RA is 0, and places EA into RA.
/// [PPC-Book1 p:42 s:3.3.3] `stwu` is an invalid form only when RA is 0, and places EA into RA.
/// [CBE-Handbook p:254 s:9.5.9] The PPE refuses a floating-point load or store with update only when RA is 0.
#[test]
fn a_valid_floating_point_load_or_store_with_update_surely_writes_ra() {
    let lfsu = PpuInstruction::Lfsu {
        frt: 4,
        ra: 4,
        imm: 8,
    };
    let stwu = PpuInstruction::Stwu {
        rs: 3,
        ra: 4,
        imm: 8,
    };
    assert_eq!(surely_written_gprs(&decoded(lfsu)), mask(&[4]));
    assert_eq!(surely_written_gprs(&decoded(stwu)), mask(&[4]));
}

/// [PPC-Book1 p:48 s:3.3.6] `lswx` loads the number of bytes that XER\[57:63\] gives, and with a count of 0 the contents of RT are undefined.
/// [PPC-Book1 p:58] For `divd` and `divw`, a divisor of 0 or a quotient that overflows leaves the contents of RT undefined.
/// [PPC-Book1 p:59] For `divdu` and `divwu`, a divisor of 0 leaves the contents of RT undefined.
/// [PPC-Book3 p:73 s:5.5.13] A System Call interrupt sets SRR0, SRR1 and MSR, and execution resumes at EA 0xC00.
#[test]
fn an_sc_an_lswx_or_a_divide_surely_writes_no_gpr() {
    let (rt, ra, rb, oe, rc) = (3, 4, 5, false, false);
    let words = [
        PpuInstruction::Sc { lev: 0 },
        PpuInstruction::Lswx { rt, ra, rb },
        PpuInstruction::Divw { rt, ra, rb, oe, rc },
        PpuInstruction::Divwu { rt, ra, rb, oe, rc },
        PpuInstruction::Divd { rt, ra, rb, oe, rc },
        PpuInstruction::Divdu { rt, ra, rb, oe, rc },
    ];
    for instruction in words {
        let decoded = decoded(instruction);
        assert_eq!(decoded, instruction);
        assert_ne!(written_gprs(&decoded), 0, "{instruction:?}");
        assert_eq!(surely_written_gprs(&decoded), 0, "{instruction:?}");
    }
}

/// [CBE-Handbook p:254 s:9.5.9] For an invalid form whose RA is in the range, the PPE completes the loads up to the collision and takes the interrupt there.
/// [PPC-Book1 p:46 s:3.3.5] `lmw` loads GPRs RT through 31.
#[test]
fn an_lmw_surely_writes_only_the_gprs_before_ra_when_ra_is_in_its_range() {
    assert_eq!(surely_written_gprs(&decoded(lmw(29, 30))), mask(&[29]));
    assert_eq!(surely_written_gprs(&decoded(lmw(30, 30))), 0);
    assert_eq!(surely_written_gprs(&decoded(lmw(5, 0))), 0);
    assert_eq!(
        surely_written_gprs(&decoded(lmw(29, 1))),
        mask(&[29, 30, 31])
    );
}

/// [CBE-Handbook p:254 s:9.5.9] For an invalid form whose RA is in the range, the PPE completes the loads up to the collision and takes the interrupt there.
/// [PPC-Book1 p:48 s:3.3.6] `lswi` loads GPRs from RT up, four bytes to each GPR, with GPR 0 after GPR 31.
#[test]
fn an_lswi_surely_writes_only_the_gprs_before_ra_when_ra_is_in_its_range() {
    assert_eq!(surely_written_gprs(&decoded(lswi(4, 5, 8))), mask(&[4]));
    assert_eq!(
        surely_written_gprs(&decoded(lswi(30, 1, 16))),
        mask(&[30, 31, 0])
    );
    assert_eq!(surely_written_gprs(&decoded(lswi(4, 9, 8))), mask(&[4, 5]));
}

/// The routine that the `bl` calls can return with any CTR. The PPE
/// refuses the `lwzu r3,0(r3)`, so r3 can still hold the copy.
///
/// [PPC-Book1 p:33 s:3.3.2] A load with update is an invalid form when RA is 0 or RT.
/// [CBE-Handbook p:254 s:9.5.9] The PPE takes the illegal-instruction interrupt for a load with update whose RA is 0 or RT.
#[test]
fn a_copy_of_ctr_after_a_call_that_crosses_an_invalid_lwzu_leaves_every_site_unresolved() {
    let between = [call(), mfctr(3), word(lwzu(3, 3)), mtctr(3)];
    assert_eq!(sites(vec![copy_stub(&between)]), UNRESOLVED);
}

/// The `lwzu r3,0(r4)` loads r3 over the copy, and the bound takes the
/// loaded CTR to hold a named address.
///
/// [PPC-Book1 p:33 s:3.3.2] A load with update whose RA is neither 0 nor RT places EA into RA and loads RT.
#[test]
fn a_valid_lwzu_over_a_copy_of_ctr_after_a_call_keeps_each_site_resolved() {
    let between = [call(), mfctr(3), word(lwzu(3, 4)), mtctr(3)];
    assert_eq!(sites(vec![copy_stub(&between)]), RESOLVED);
}

/// `lmw r29,0(r30)` loads r29 before the collision at r30, so the copies
/// in r30 and r31 can survive, and the copy in r29 does not.
///
/// [CBE-Handbook p:254 s:9.5.9] For an invalid form whose RA is in the range, the PPE completes the loads up to the collision and takes the interrupt there.
#[test]
fn an_invalid_lmw_keeps_a_copy_of_ctr_at_and_after_ra_and_removes_it_before_ra() {
    let at_ra = [call(), mfctr(30), word(lmw(29, 30)), mtctr(30)];
    assert_eq!(sites(vec![copy_stub(&at_ra)]), UNRESOLVED);
    let after_ra = [call(), mfctr(31), word(lmw(29, 30)), mtctr(31)];
    assert_eq!(sites(vec![copy_stub(&after_ra)]), UNRESOLVED);
    let before_ra = [call(), mfctr(29), word(lmw(29, 30)), mtctr(29)];
    assert_eq!(sites(vec![copy_stub(&before_ra)]), RESOLVED);
}

/// `lswi r4,r5,8` loads r4 before the collision at r5, so the copy in r5
/// can survive, and the copy in r4 does not.
///
/// [CBE-Handbook p:254 s:9.5.9] For an invalid form whose RA is in the range, the PPE completes the loads up to the collision and takes the interrupt there.
#[test]
fn an_invalid_lswi_keeps_a_copy_of_ctr_at_ra_and_removes_it_before_ra() {
    let at_ra = [call(), mfctr(5), word(lswi(4, 5, 8)), mtctr(5)];
    assert_eq!(sites(vec![copy_stub(&at_ra)]), UNRESOLVED);
    let before_ra = [call(), mfctr(4), word(lswi(4, 5, 8)), mtctr(4)];
    assert_eq!(sites(vec![copy_stub(&before_ra)]), RESOLVED);
}

/// r5 holds a copy of the CTR that the routine returns, and the slot at
/// 20(r1) holds the low word of a built address. The PPE refuses the
/// `lmw r0,0(r1)` after it loads r0, so r5 can hold either copy.
///
/// [CBE-Handbook p:254 s:9.5.9] For an invalid form whose RA is in the range, the PPE completes the loads up to the collision and takes the interrupt there.
/// [PPC-Book1 p:42 s:3.3.3] `stw` stores RS 32:63 into the word at EA.
#[test]
fn an_invalid_lmw_joins_a_slot_copy_into_a_gpr_past_ra_that_holds_a_copy() {
    let stw = word(PpuInstruction::Stw {
        rs: 4,
        ra: 1,
        imm: 20,
    });
    let mut between = vec![call(), mfctr(5)];
    between.extend(build(0x300C));
    between.extend([mtctr(3), mfctr(4), stw, word(lmw(0, 1)), mtctr(5)]);
    assert_eq!(sites(vec![copy_stub(&between)]), UNRESOLVED);
}

/// The 65 writers bring more constants in LR than a raised bound keeps,
/// so the `mflr` copy in r5 can hold a dropped constant. The PPE
/// refuses the `lwzu r5,0(r5)`, so r5 keeps that mark.
///
/// [PPC-Book1 p:33 s:3.3.2] A load with update is an invalid form when RA is 0 or RT.
/// [CBE-Handbook p:254 s:9.5.9] The PPE takes the illegal-instruction interrupt for a load with update whose RA is 0 or RT.
#[test]
fn a_bctr_of_an_mflr_copy_of_a_lost_lr_across_an_invalid_lwzu_leaves_every_site_unresolved() {
    let mut words = lr_writers(RAISED_LIMIT + 1);
    words.extend([mflr(5), word(lwzu(5, 5)), join_mtctr(5), join_bctr()]);
    assert_eq!(join_sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The `lwzu r5,0(r4)` loads r5 over the `mflr` copy of the lost LR.
///
/// [PPC-Book1 p:33 s:3.3.2] A load with update whose RA is neither 0 nor RT places EA into RA and loads RT.
#[test]
fn a_bctr_of_a_valid_lwzu_over_an_mflr_copy_of_a_lost_lr_keeps_the_sites_resolved() {
    let mut words = lr_writers(RAISED_LIMIT + 1);
    words.extend([mflr(5), word(lwzu(5, 4)), join_mtctr(5), join_bctr()]);
    assert_eq!(join_sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// The handler of the `sc` can return r5 unchanged, so r5 keeps the mark
/// of the `mflr` copy of the lost LR.
///
/// [PPC-Book3 p:73 s:5.5.13] A System Call interrupt sets SRR0, SRR1 and MSR, and execution resumes at EA 0xC00.
#[test]
fn a_bctr_of_an_mflr_copy_of_a_lost_lr_across_an_sc_leaves_every_site_unresolved() {
    let sc = word(PpuInstruction::Sc { lev: 0 });
    let mut words = lr_writers(RAISED_LIMIT + 1);
    words.extend([mflr(5), sc, join_mtctr(5), join_bctr()]);
    assert_eq!(join_sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// [CBE-Handbook p:254 s:9.5.9] For an invalid form whose RA is in the range, the PPE completes the loads up to the collision and takes the interrupt there.
/// [PPC-Book1 p:48 s:3.3.6] `lswi` with NB 0 loads 32 bytes into GPRs from RT up, four bytes to each GPR, with GPR 0 after GPR 31.
#[test]
fn an_lswi_of_32_bytes_surely_writes_only_the_gprs_before_ra_when_ra_is_in_its_range() {
    assert_eq!(
        surely_written_gprs(&decoded(lswi(30, 2, 0))),
        mask(&[30, 31, 0, 1])
    );
}

/// r5 holds no copy, and the slot at 20(r1) holds a copy of the CTR
/// that the routine returns. The PPE refuses the `lmw r0,0(r1)` after it
/// loads r0. The `lmw` does not surely write r5, and a run that loads r5
/// takes the copy from the slot, so r5 can hold the copy.
///
/// [CBE-Handbook p:254 s:9.5.9] For an invalid form whose RA is in the range, the PPE completes the loads up to the collision and takes the interrupt there.
#[test]
fn an_invalid_lmw_joins_a_slot_copy_into_a_gpr_past_ra_that_holds_no_copy() {
    let stw = word(PpuInstruction::Stw {
        rs: 4,
        ra: 1,
        imm: 20,
    });
    let between = [call(), mfctr(4), stw, word(lmw(0, 1)), mtctr(5)];
    assert_eq!(sites(vec![copy_stub(&between)]), UNRESOLVED);
}
