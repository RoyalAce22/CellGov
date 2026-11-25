use super::hypercall_ctr_copy_tests::{after_bdnz_falls_through, call, copy_stub, mfctr, mtctr};
use super::hypercall_ctr_decrement_tests::QUIET_SC;
use super::hypercall_ctr_stack_copy_tests::{
    after_a_load_of_part_of_a_copy, after_a_routine_that_returns_a_slot, RESOLVED, UNRESOLVED,
};
use super::hypercall_ctr_virtual_tests::sites;
use super::hypercall_entry_tests::word;
use super::hypercall_tests::li;
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

/// `stw rs,imm(ra)`.
fn stw(rs: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Stw { rs, ra, imm })
}

/// `addi rt,ra,imm`.
fn addi(rt: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Addi { rt, ra, imm })
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

/// r31 holds the address in r1, so the `std` through r31 writes the
/// doubleword that the `ld` through r1 reads.
///
/// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
#[ignore = "the CTR flow keeps no copy in memory that no stack slot names; one cell for that memory left every site of a held kernel unresolved"]
fn a_copy_of_ctr_stored_through_a_copy_of_r1_and_loaded_through_r1_leaves_every_site_unresolved() {
    let between = [mr(31, 1), std(0, 31, 16), ld(0, 1, 16)];
    assert_eq!(after_a_call_copies_ctr(&between), UNRESOLVED);
}

/// r31 holds the address in r1 plus 16, so the `ld` at 0(r31) reads the
/// doubleword that the `std` at 16(r1) writes.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0, and places the sum into RT.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
#[ignore = "the CTR flow keeps no copy in memory that no stack slot names, and a load through such an address takes no slot copy; one cell for that memory left every site of a held kernel unresolved"]
fn a_copy_of_ctr_stored_through_r1_and_loaded_through_a_frame_pointer_leaves_every_site_unresolved()
{
    let between = [addi(31, 1, 16), std(0, 1, 16), ld(0, 31, 0)];
    assert_eq!(after_a_call_copies_ctr(&between), UNRESOLVED);
}

/// The indexed forms add r9 to r1, so `stdx` and `ldx` reach the
/// doubleword at 16(r1).
///
/// [PPC-Book1 p:43 s:3.3.3] `stdx` stores RS into the doubleword at EA = (RA|0) + (RB).
/// [PPC-Book1 p:39 s:3.3.2] `ldx` loads the doubleword at EA = (RA|0) + (RB) into RT.
#[test]
#[ignore = "the CTR flow keeps no copy in memory that no stack slot names, and a load through such an address takes no slot copy; one cell for that memory left every site of a held kernel unresolved"]
fn a_copy_of_ctr_through_an_indexed_store_or_load_leaves_every_site_unresolved() {
    let stdx = word(PpuInstruction::Stdx {
        rs: 0,
        ra: 1,
        rb: 9,
    });
    let ldx = word(PpuInstruction::Ldx {
        rt: 0,
        ra: 1,
        rb: 9,
    });
    assert_eq!(
        after_a_call_copies_ctr(&[li(9, 16), stdx, ld(0, 1, 16)]),
        UNRESOLVED
    );
    assert_eq!(
        after_a_call_copies_ctr(&[std(0, 1, 16), li(9, 16), ldx]),
        UNRESOLVED
    );
}

/// The `bdnz` falls through only when CTR is 0. The scan keeps no
/// address for the `stdx`, so the `ld` gets a value that the scan cannot
/// prove in place of the 0.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
/// [PPC-Book1 p:43 s:3.3.3] `stdx` stores RS into the doubleword at EA = (RA|0) + (RB).
#[test]
#[ignore = "the CTR flow keeps no copy in memory that no stack slot names; one cell for that memory left every site of a held kernel unresolved"]
fn a_copy_of_a_ctr_of_zero_through_an_indexed_store_leaves_every_site_unresolved() {
    let stdx = word(PpuInstruction::Stdx {
        rs: 0,
        ra: 1,
        rb: 9,
    });
    assert_eq!(
        after_bdnz_falls_through(&[mfctr(0), li(9, 16), stdx, ld(0, 1, 16), mtctr(0)]),
        [(4, None), (QUIET_SC, None)]
    );
}

/// The `ld r1,0(r1)` writes r1 with a value that the scan does not
/// know, so the `ld` at 16(r1) can read the copy at 128(r1).
///
/// [PPC-Book1 p:43 s:3.3.3] `stdu` stores RS at EA = (RA) + EXTS(DS || 0b00), then places EA into RA.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
#[ignore = "the CTR flow keeps no copy in memory that no stack slot names; one cell for that memory left every site of a held kernel unresolved"]
fn a_copy_of_ctr_in_a_slot_after_a_reload_of_r1_from_memory_leaves_every_site_unresolved() {
    let stdu = word(PpuInstruction::Stdu {
        rs: 1,
        ra: 1,
        imm: -112,
    });
    let between = [stdu, std(0, 1, 128), ld(1, 1, 0), ld(0, 1, 16)];
    assert_eq!(after_a_call_copies_ctr(&between), UNRESOLVED);
}

/// The `mr r1,r31` writes r1 from another GPR, so the `ld` at 16(r1) can
/// read the copy at 128(r1).
///
/// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
/// [PPC-Book1 p:43 s:3.3.3] `stdu` stores RS at EA = (RA) + EXTS(DS || 0b00), then places EA into RA.
#[test]
#[ignore = "the CTR flow keeps no copy in memory that no stack slot names; one cell for that memory left every site of a held kernel unresolved"]
fn a_copy_of_ctr_in_a_slot_after_a_move_of_r1_from_another_gpr_leaves_every_site_unresolved() {
    let stdu = word(PpuInstruction::Stdu {
        rs: 1,
        ra: 1,
        imm: -112,
    });
    let between = [mr(31, 1), stdu, std(0, 1, 128), mr(1, 31), ld(0, 1, 16)];
    assert_eq!(after_a_call_copies_ctr(&between), UNRESOLVED);
}

/// Two moves of r1 down by 32000 take the copy at 16(r1) to 64016(r1),
/// past the 16-bit displacement, and two moves up bring it back.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0, and places the sum into RT.
/// [PPC-Book1 p:37 s:3.3.2] `lwz` computes EA as (RA|0) + EXTS(D), with D a 16-bit field.
#[test]
#[ignore = "the CTR flow keeps no copy in memory that no stack slot names; one cell for that memory left every site of a held kernel unresolved"]
fn a_copy_of_ctr_in_a_slot_that_a_move_takes_out_of_reach_leaves_every_site_unresolved() {
    let between = [
        std(0, 1, 16),
        addi(1, 1, -32000),
        addi(1, 1, -32000),
        addi(1, 1, 32000),
        addi(1, 1, 32000),
        ld(0, 1, 16),
    ];
    assert_eq!(after_a_call_copies_ctr(&between), UNRESOLVED);
}

/// A routine stores its CTR through a copy of r1 and returns, so the
/// slot at 16(r1) can hold that CTR after the call.
///
/// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:25 s:2.4] `bclr` branches to LR 0:61 || 0b00.
#[test]
#[ignore = "the CTR flow keeps no copy in memory that no stack slot names; one cell for that memory left every site of a held kernel unresolved"]
fn a_real_mode_bctr_of_a_ctr_a_routine_returns_through_a_copy_of_r1_leaves_every_site_unresolved() {
    assert_eq!(
        after_a_routine_that_returns_a_slot(&[mfctr(5), mr(31, 1), std(5, 31, 16), li(5, 0)]),
        UNRESOLVED
    );
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
/// every site is unresolved. After the `bdnz` falls through, CTR is 0, so
/// the `bctr` reaches address 0.
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
/// through r31 and then r0, so r0 gets the word at 28(r1), the low word
/// 0x00008000 of the copy at 24(r1).
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
/// 0x80000000, which no segment holds, so the `bctr` reaches neither
/// address 0 nor address 0x8000. `lswi r31,r1,6` loads the high word
/// into r31 and the next two bytes, 0x0000, at the left of the low word
/// of r0, so the `bctr` reaches address 0.
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

/// The string forms through r31 or r9 reach memory at an address that
/// the scan does not know, and the `lwz` at 0(r1) can read it.
///
/// [PPC-Book1 p:49 s:3.3.6] `stswi` stores n bytes from the low-order words of RS onward, left to right, to EA = (RA|0).
/// [PPC-Book1 p:48 s:3.3.6] `lswi` loads n bytes from EA = (RA|0) into the low-order words of RT onward, left to right, with GPR 0 after GPR 31, and sets the high-order word and the unfilled bytes to 0.
#[test]
#[ignore = "the CTR flow keeps no copy in memory that no stack slot names, and a load through such an address takes no slot copy; one cell for that memory left every site of a held kernel unresolved"]
fn a_copy_of_ctr_through_a_string_form_with_another_base_leaves_every_site_unresolved() {
    assert_eq!(
        after_a_call_copies_ctr(&[mr(31, 1), stswi(0, 31, 4), lwz(0, 1, 0)]),
        UNRESOLVED
    );
    let words = [call(), mfctr(0), stw(0, 1, 0), lswi(30, 9, 8), mtctr(31)];
    assert_eq!(sites(vec![copy_stub(&words)]), UNRESOLVED);
}

/// `stswx` and `lswx` take their byte count from XER and add RB to RA,
/// so they can store or load a copy at any address.
///
/// [PPC-Book1 p:49 s:3.3.6] `stswx` stores the number of bytes that XER\[57:63\] gives, from RS onward, to EA = (RA|0) + (RB).
/// [PPC-Book1 p:48 s:3.3.6] `lswx` loads the number of bytes that XER\[57:63\] gives, into GPRs from RT up.
#[test]
#[ignore = "the CTR flow keeps no copy in memory that no stack slot names, and a load through such an address takes no slot copy; one cell for that memory left every site of a held kernel unresolved"]
fn a_copy_of_ctr_through_an_indexed_string_form_leaves_every_site_unresolved() {
    let stswx = word(PpuInstruction::Stswx {
        rs: 0,
        ra: 1,
        rb: 9,
    });
    let lswx = word(PpuInstruction::Lswx {
        rt: 30,
        ra: 9,
        rb: 10,
    });
    assert_eq!(after_a_call_copies_ctr(&[stswx, lwz(0, 1, 0)]), UNRESOLVED);
    let words = [call(), mfctr(0), std(0, 1, 16), lswx, mtctr(31)];
    assert_eq!(sites(vec![copy_stub(&words)]), UNRESOLVED);
}
