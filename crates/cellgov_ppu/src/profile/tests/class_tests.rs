//! Each operand class splits at its boundary and nowhere else.

use super::{BaseClass, InsnClass, PairClass};

/// `lwz rT, d(rA)`.
fn lwz(rt: u32, ra: u32, d: u16) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

/// `std rS, d(rA)`, `d` a multiple of four.
fn std(rs: u32, ra: u32, d: u16) -> u32 {
    (62 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

/// `cmpwi crF, rA, imm`.
fn cmpwi(bf: u32, ra: u32, imm: u16) -> u32 {
    (11 << 26) | (bf << 23) | (ra << 16) | u32::from(imm)
}

/// `rlwinm[.] rA, rS, sh, mb, me`.
fn rlwinm(ra: u32, rs: u32, sh: u32, rc: bool) -> u32 {
    (21 << 26) | (rs << 21) | (ra << 16) | (sh << 11) | (31 << 1) | u32::from(rc)
}

/// `ori rA, rS, imm`.
fn ori(ra: u32, rs: u32, imm: u16) -> u32 {
    (24 << 26) | (rs << 21) | (ra << 16) | u32::from(imm)
}

/// `bc BO, BI, 8`.
fn bc(bo: u32, bi: u32) -> u32 {
    (16 << 26) | (bo << 21) | (bi << 16) | 8
}

fn class(word: u32) -> InsnClass {
    InsnClass::of(word).expect("the word decodes")
}

#[test]
fn a_base_of_r0_is_the_literal_zero_and_r1_its_own_class() {
    assert_eq!(class(lwz(3, 0, 0)).base, Some(BaseClass::Zero));
    assert_eq!(class(lwz(3, 1, 0)).base, Some(BaseClass::R1));
    assert_eq!(class(lwz(3, 2, 0)).base, Some(BaseClass::R2));
    assert_eq!(class(lwz(3, 13, 0)).base, Some(BaseClass::R13));
    assert_eq!(class(lwz(3, 9, 0)).base, Some(BaseClass::Other));
    assert_eq!(class(cmpwi(0, 3, 0)).base, None);
}

#[test]
fn the_rc_bit_splits_a_rotate_into_two_classes() {
    let plain = class(rlwinm(3, 4, 8, false));
    let record = class(rlwinm(3, 4, 8, true));
    assert_eq!(plain.rc, Some(false));
    assert_eq!(record.rc, Some(true));
    assert_ne!(plain, record);
}

#[test]
fn a_form_without_an_rc_bit_has_no_rc_class() {
    // `mflr r0`: mfspr, whose bit 31 is reserved.
    let mflr = (31 << 26) | (8 << 16) | (339 << 1);
    assert_eq!(class(mflr).rc, None);
    // `or r3, r4, r5`, then `or.`.
    let or = (31 << 26) | (4 << 21) | (3 << 16) | (5 << 11) | (444 << 1);
    assert_eq!(class(or).rc, Some(false));
    assert_eq!(class(or | 1).rc, Some(true));
    // `add r3, r4, r5` and `addo r3, r4, r5`: OE does not hide the form.
    let add = (31 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (266 << 1);
    assert_eq!(class(add).rc, Some(false));
    assert_eq!(class(add | (1 << 10)).rc, Some(false));
}

#[test]
fn a_zero_immediate_is_its_own_class() {
    assert_eq!(class(cmpwi(0, 3, 0)).imm_zero, Some(true));
    assert_eq!(class(cmpwi(0, 3, 1)).imm_zero, Some(false));
    assert_eq!(class(cmpwi(0, 3, 0xFFFF)).imm_zero, Some(false));
}

#[test]
fn a_same_register_shape_is_its_own_class() {
    assert_eq!(class(ori(3, 3, 0)).same_reg, Some(true));
    assert_eq!(class(ori(3, 4, 0)).same_reg, Some(false));
}

#[test]
fn the_hint_bits_of_one_bo_class_give_one_key() {
    // 011at: branch if the CR bit is set.
    let set: Vec<_> = (12..16).map(|bo| class(bc(bo, 2))).collect();
    assert!(set.iter().all(|c| *c == set[0]), "{set:?}");
    assert_eq!(set[0].bo, Some(0b01100));
    // 001at: branch if the CR bit is clear.
    assert_eq!(class(bc(4, 2)).bo, class(bc(7, 2)).bo);
    assert_ne!(class(bc(4, 2)), set[0]);
    // 1a00t: decrement CTR, branch if it is not zero.
    assert_eq!(class(bc(16, 0)).bo, Some(0b10000));
    assert_eq!(class(bc(25, 0)).bo, Some(0b10000));
    // 1a01t: decrement CTR, branch if it is zero.
    assert_eq!(class(bc(18, 0)).bo, Some(0b10010));
    // 1z1zz: branch always.
    assert_eq!(class(bc(20, 0)).bo, Some(0b10100));
}

#[test]
fn a_pair_whose_second_reads_the_first_target_is_dependent() {
    let dependent = PairClass::of(lwz(3, 1, 0), cmpwi(0, 3, 0)).unwrap();
    assert!(dependent.dependent);
    let independent = PairClass::of(lwz(3, 1, 0), cmpwi(0, 4, 0)).unwrap();
    assert!(!independent.dependent);
}

#[test]
fn two_stores_off_one_base_are_adjacent_only_a_doubleword_apart() {
    let adjacent = PairClass::of(std(6, 1, 0), std(7, 1, 8)).unwrap();
    assert!(adjacent.shared_base && adjacent.adjacent);
    let apart = PairClass::of(std(6, 1, 0), std(7, 1, 16)).unwrap();
    assert!(apart.shared_base && !apart.adjacent);
    let other_base = PairClass::of(std(6, 1, 0), std(7, 2, 8)).unwrap();
    assert!(!other_base.shared_base && !other_base.adjacent);
}

#[test]
fn the_multiplies_and_sradi_have_an_rc_class() {
    // `mulld r3, r4, r5`, `mulhwu r3, r4, r5`, `sradi r3, r4, 63`.
    let mulld = (31 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (233 << 1);
    let mulhwu = (31 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (11 << 1);
    let sradi = (31 << 26) | (4 << 21) | (3 << 16) | (31 << 11) | (413 << 2) | (1 << 1);
    for word in [mulld, mulhwu, sradi] {
        assert_eq!(class(word).rc, Some(false), "{word:#010x}");
        assert_eq!(class(word | 1).rc, Some(true), "{word:#010x}");
    }
}

#[test]
fn each_floating_point_operation_is_its_own_class_and_a_compare_has_no_rc() {
    // `fadd f1, f2, f3`, `fmul f1, f2, f3`, `fcmpu cr0, f1, f2`.
    let fadd = (63 << 26) | (1 << 21) | (2 << 16) | (3 << 11) | (21 << 1);
    let fmul = (63 << 26) | (1 << 21) | (2 << 16) | (3 << 6) | (25 << 1);
    let fcmpu = (63 << 26) | (1 << 16) | (2 << 11);
    assert_eq!(class(fadd).name, "fadd");
    assert_eq!(class(fmul).name, "fmul");
    assert_eq!(class(fadd).rc, Some(false));
    assert_eq!(class(fadd | 1).rc, Some(true));
    assert_eq!(class(fcmpu).rc, None);
}

#[test]
fn a_pair_whose_second_only_overwrites_the_first_target_is_not_dependent() {
    // `add r3, r4, r5` writes r3 and reads r4 and r5.
    let add = |rt: u32, ra: u32| (31 << 26) | (rt << 21) | (ra << 16) | (5 << 11) | (266 << 1);
    assert!(!PairClass::of(lwz(3, 1, 0), add(3, 4)).unwrap().dependent);
    assert!(PairClass::of(lwz(3, 1, 0), add(4, 3)).unwrap().dependent);
    // `or r3, r4, r4` writes r3 and reads r4.
    let or = (31 << 26) | (4 << 21) | (3 << 16) | (4 << 11) | (444 << 1);
    assert!(!PairClass::of(lwz(3, 1, 0), or).unwrap().dependent);
    // `stfd f3, 8(r1)` stores an FPR.
    let stfd = (54 << 26) | (3 << 21) | (1 << 16) | 8;
    assert!(!PairClass::of(lwz(3, 1, 0), stfd).unwrap().dependent);
}

#[test]
fn a_rotate_by_register_or_an_insert_reads_the_register_it_names() {
    // `rlwnm r3, r4, r5, 0, 31` and `rldcl r3, r4, r5, 0` rotate by r5.
    let rlwnm = (23 << 26) | (4 << 21) | (3 << 16) | (5 << 11) | (31 << 1);
    let rldcl = (30 << 26) | (4 << 21) | (3 << 16) | (5 << 11) | (8 << 1);
    for word in [rlwnm, rldcl] {
        assert!(
            PairClass::of(lwz(5, 1, 0), word).unwrap().dependent,
            "{word:#010x}"
        );
        assert!(
            !PairClass::of(lwz(6, 1, 0), word).unwrap().dependent,
            "{word:#010x}"
        );
    }
    // `rldimi r3, r4, 0, 0` merges into the old r3.
    let rldimi = (30 << 26) | (4 << 21) | (3 << 16) | (3 << 2);
    assert!(PairClass::of(lwz(3, 1, 0), rldimi).unwrap().dependent);
    // `rldicl r3, r4, 0, 0` reads only r4.
    let rldicl = (30 << 26) | (4 << 21) | (3 << 16);
    assert!(!PairClass::of(lwz(3, 1, 0), rldicl).unwrap().dependent);
}

#[test]
fn an_spr_number_or_a_compare_field_is_not_a_register_read() {
    // `mtlr r0`: the LR number 8 sits in the RA slot.
    let mtlr = (31 << 26) | (8 << 16) | (467 << 1);
    assert!(!PairClass::of(lwz(8, 1, 0), mtlr).unwrap().dependent);
    assert!(PairClass::of(lwz(0, 1, 0), mtlr).unwrap().dependent);
    // `mtcrf 0xFF, r3`: the FXM mask reads as 15 in the RA slot.
    let mtcrf = (31 << 26) | (3 << 21) | (0xFF << 12) | (144 << 1);
    assert!(!PairClass::of(lwz(15, 1, 0), mtcrf).unwrap().dependent);
    assert!(PairClass::of(lwz(3, 1, 0), mtcrf).unwrap().dependent);
    // `cmpw cr7, r3, r4`: BF 7 and L 0 read as 28 in the RT slot.
    let cmpw = (31 << 26) | (28 << 21) | (3 << 16) | (4 << 11);
    assert!(!PairClass::of(lwz(28, 1, 0), cmpw).unwrap().dependent);
    assert!(PairClass::of(lwz(4, 1, 0), cmpw).unwrap().dependent);
}

#[test]
fn a_bo_that_decrements_and_tests_the_condition_drops_only_its_z_bit() {
    // 0000z and 0100z: decrement CTR, branch if CTR != 0 and the CR bit
    // is clear (set).
    assert_eq!(class(bc(0, 2)).bo, Some(0b00000));
    assert_eq!(class(bc(1, 2)).bo, Some(0b00000));
    assert_eq!(class(bc(9, 2)).bo, Some(0b01000));
    // 0001z: decrement CTR, branch if CTR == 0 and the CR bit is clear.
    assert_eq!(class(bc(3, 2)).bo, Some(0b00010));
}

#[test]
fn a_branch_to_lr_or_ctr_has_bo_and_lk_and_no_aa() {
    // `blr`, `bctrl`, and `bltlr` with both hint bits set.
    let bclr = |bo: u32, lk: u32| (19 << 26) | (bo << 21) | (16 << 1) | lk;
    let bcctr = |bo: u32, lk: u32| (19 << 26) | (bo << 21) | (528 << 1) | lk;
    assert_eq!(class(bclr(20, 0)).label(), "Bclr bo=10100 lk=0");
    assert_eq!(class(bcctr(20, 1)).label(), "Bcctr bo=10100 lk=1");
    assert_eq!(class(bclr(15, 0)).bo, Some(0b01100));
}

#[test]
fn a_label_names_the_variant_and_each_class_it_has() {
    assert_eq!(class(lwz(3, 1, 0)).label(), "Lwz base=r1 distinct");
    assert_eq!(
        class(cmpwi(0, 3, 0)).label(),
        "Cmpwi imm=0 distinct",
        "pins today's same-register class on a compare, whose BF and L field names no GPR"
    );
    assert_eq!(class(bc(13, 2)).label(), "Bc bo=01100 aa=0 lk=0");
}
