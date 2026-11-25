use super::hypercall_ctr_copy_tests::{after_bdnz_falls_through, call, copy_stub, mfctr, mtctr};
use super::hypercall_ctr_decrement_tests::QUIET_SC;
use super::hypercall_ctr_stack_copy_tests::{after_a_load_of_part_of_a_copy, RESOLVED, UNRESOLVED};
use super::hypercall_ctr_virtual_tests::sites;
use super::hypercall_entry_tests::word;
use super::*;

/// `mr ra,rs`, as `or ra,rs,rs`.
fn mr(ra: u8, rs: u8) -> u32 {
    word(PpuInstruction::Or {
        ra,
        rs,
        rb: rs,
        rc: false,
    })
}

/// `std rs,imm(ra)`.
fn std(rs: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Std { rs, ra, imm })
}

/// `ld rt,imm(ra)`.
fn ld(rt: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Ld { rt, ra, imm })
}

/// `lwz rt,imm(ra)`.
fn lwz(rt: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Lwz { rt, ra, imm })
}

/// `lhz rt,imm(ra)`.
fn lhz(rt: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Lhz { rt, ra, imm })
}

/// `stswi rs,ra,nb`.
fn stswi(rs: u8, ra: u8, nb: u8) -> u32 {
    word(PpuInstruction::Stswi { rs, ra, nb })
}

/// `lswi rt,ra,nb`.
fn lswi(rt: u8, ra: u8, nb: u8) -> u32 {
    word(PpuInstruction::Lswi { rt, ra, nb })
}

/// The sites when a stub copies a CTR that a call leaves at any value
/// into r0, runs `between`, and moves r0 into CTR.
fn after_a_call_copies_ctr(between: &[u32]) -> Vec<(u64, Option<u16>)> {
    let mut words = vec![call(), mfctr(0)];
    words.extend_from_slice(between);
    words.push(mtctr(0));
    sites(vec![copy_stub(&words)])
}

/// The stores through r31 write no copy of CTR, and the loads through
/// r9 and r1 replace the copy in r0.
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
fn a_store_without_a_copy_of_ctr_and_a_load_through_another_base_keep_each_site_resolved() {
    assert_eq!(
        after_a_call_copies_ctr(&[mr(31, 1), std(4, 31, 16), ld(0, 1, 16)]),
        RESOLVED
    );
    assert_eq!(after_a_call_copies_ctr(&[ld(0, 9, 0)]), RESOLVED);
}

/// The `stfd` writes an FPR over the copy of CTR at 16(r1), so the `ld`
/// loads no copy. The `lfd` reads the slot and leaves the copy there.
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:108 s:4.6.3] `stfd` stores FRS into the doubleword at EA.
/// [PPC-Book1 p:105 s:4.6.2] `lfd` loads the doubleword at EA into FRT.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
fn a_copy_of_ctr_in_a_slot_goes_with_an_fpr_store_and_stays_through_an_fpr_load() {
    let stfd = word(PpuInstruction::Stfd {
        frs: 1,
        ra: 1,
        imm: 16,
    });
    let lfd = word(PpuInstruction::Lfd {
        frt: 1,
        ra: 1,
        imm: 16,
    });
    assert_eq!(
        after_a_call_copies_ctr(&[std(0, 1, 16), stfd, ld(0, 1, 16)]),
        RESOLVED
    );
    assert_eq!(
        after_a_call_copies_ctr(&[std(0, 1, 16), lfd, ld(0, 1, 16)]),
        UNRESOLVED
    );
}

/// `stswi` stores the low word of r0 at 0(r1), and `lwz` loads it back.
///
/// [PPC-Book1 p:49 s:3.3.6] `stswi` stores n bytes from the low-order words of RS onward, left to right, to EA = (RA|0).
/// [PPC-Book1 p:37 s:3.3.2] `lwz` loads the word at EA into RT 32:63 and sets RT 0:31 to 0.
#[test]
fn a_copy_of_ctr_through_a_string_store_leaves_every_site_unresolved() {
    let between = [stswi(0, 1, 4), lwz(0, 1, 0)];
    assert_eq!(after_a_call_copies_ctr(&between), UNRESOLVED);
}

/// CTR holds 0x8000000000008000. `stswi` of 4 bytes stores its low word
/// 0x00008000 at 0(r1), so the `bctr` reaches address 0x8000.
///
/// [PPC-Book1 p:49 s:3.3.6] `stswi` stores n bytes from the low-order words of RS onward, left to right, to EA = (RA|0).
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:37 s:3.3.2] `lwz` loads the word at EA into RT 32:63 and sets RT 0:31 to 0.
#[test]
fn a_string_store_of_a_word_of_a_built_ctr_enters_that_word() {
    assert_eq!(
        after_a_load_of_part_of_a_copy(0x8000, 0x8000, &[stswi(0, 1, 4), lwz(0, 1, 0)]),
        [(0x8004, None), (QUIET_SC, Some(77))]
    );
}

/// CTR holds 0x8000000000008000. `stswi` of 2 bytes stores the two
/// high-order bytes of its low word, 0x0000, so the `bctr` reaches
/// address 0 and never 0x8000. `stswi` of 3 bytes stores 0x00, 0x00 and
/// 0x80, so the `lhz` at 0(r1) also loads 0.
///
/// [PPC-Book1 p:49 s:3.3.6] `stswi` stores n bytes from the low-order words of RS onward, left to right, to EA = (RA|0).
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:35 s:3.3.2] `lhz` loads the halfword at EA into RT 48:63 and sets RT 0:47 to 0.
#[test]
fn a_string_store_of_part_of_a_word_stores_its_high_order_bytes() {
    for nb in [2, 3] {
        let between = [stswi(0, 1, nb), lhz(0, 1, 0)];
        assert_eq!(
            after_a_load_of_part_of_a_copy(0, 0x8000, &between),
            [(4, None), (QUIET_SC, Some(77))],
            "{nb} bytes"
        );
        assert_eq!(
            after_a_load_of_part_of_a_copy(0x8000, 0x8000, &between),
            [(0x8004, Some(75)), (QUIET_SC, Some(77))],
            "{nb} bytes"
        );
    }
}

/// `lswi` of 8 bytes loads the high word of the copy at 0(r1) into r30
/// and its low word into r31. After a call the copy can hold any CTR, so
/// every site stays unresolved. After the `bdnz` falls through, CTR is 0,
/// so the `bctr` reaches address 0.
///
/// [PPC-Book1 p:48 s:3.3.6] `lswi` loads n bytes from EA = (RA|0) into the low-order words of RT onward, left to right, with GPR 0 after GPR 31, and sets the high-order word and the unfilled bytes to 0.
#[test]
fn a_copy_of_ctr_through_a_string_load_brings_the_copied_ctr_to_the_bctr() {
    let mut words = vec![call(), mfctr(0), std(0, 1, 0), lswi(30, 1, 8), mtctr(31)];
    assert_eq!(sites(vec![copy_stub(&words)]), UNRESOLVED);
    words.splice(0..1, []);
    assert_eq!(
        after_bdnz_falls_through(&words),
        [(4, None), (QUIET_SC, Some(77))]
    );
}

/// CTR holds 0x8000000000008000. `lswi r31,r1,8` loads its high word
/// into r31 and, after GPR 31, its low word 0x00008000 into r0.
///
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:48 s:3.3.6] `lswi` loads n bytes from EA = (RA|0) into the low-order words of RT onward, left to right, with GPR 0 after GPR 31, and sets the high-order word and the unfilled bytes to 0.
#[test]
fn a_string_load_that_wraps_to_r0_loads_the_low_word_of_a_built_ctr_into_r0() {
    assert_eq!(
        after_a_load_of_part_of_a_copy(0x8000, 0x8000, &[std(0, 1, 0), lswi(31, 1, 8)]),
        [(0x8004, None), (QUIET_SC, Some(77))]
    );
}

/// CTR holds 0x8000000000008000. `lswi r25,r1,0` loads 32 bytes into r25
/// through r31 and then r0. So r0 gets the word at 28(r1), which is the
/// low word 0x00008000 of the copy at 24(r1).
///
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:48 s:3.3.6] `lswi` with NB = 0 loads 32 bytes, into the low-order words of RT onward, with GPR 0 after GPR 31.
#[test]
fn a_string_load_with_a_byte_count_of_zero_loads_32_bytes() {
    assert_eq!(
        after_a_load_of_part_of_a_copy(0x8000, 0x8000, &[std(0, 1, 24), lswi(25, 1, 0)]),
        [(0x8004, None), (QUIET_SC, Some(77))]
    );
}

/// CTR holds 0x8000000000008000, so the bytes at 0(r1) are 0x80 and
/// 0x00. `lswi` of 2 bytes puts them at the left of the low word of r0,
/// which gives 0x80000000. No segment holds that address, so the `bctr`
/// reaches neither address 0 nor address 0x8000. `lswi r31,r1,6` loads
/// the high word into r31. It puts the next two bytes, 0x0000, at the
/// left of the low word of r0, so the `bctr` reaches address 0.
///
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:48 s:3.3.6] `lswi` loads n bytes from EA = (RA|0) into the low-order words of RT onward, left to right, with GPR 0 after GPR 31, and sets the high-order word and the unfilled bytes to 0.
#[test]
fn a_string_load_of_part_of_a_word_puts_its_bytes_at_the_left_of_the_low_word() {
    let between = [std(0, 1, 0), lswi(0, 1, 2)];
    assert_eq!(
        after_a_load_of_part_of_a_copy(0, 0x8000, &between),
        [(4, Some(75)), (QUIET_SC, Some(77))]
    );
    assert_eq!(
        after_a_load_of_part_of_a_copy(0x8000, 0x8000, &between),
        [(0x8004, Some(75)), (QUIET_SC, Some(77))]
    );
    let after_a_word = [std(0, 1, 0), lswi(31, 1, 6)];
    assert_eq!(
        after_a_load_of_part_of_a_copy(0, 0x8000, &after_a_word),
        [(4, None), (QUIET_SC, Some(77))]
    );
    assert_eq!(
        after_a_load_of_part_of_a_copy(0x8000, 0x8000, &after_a_word),
        [(0x8004, Some(75)), (QUIET_SC, Some(77))]
    );
}
