use super::hypercall_ctr_copy_tests::{after_bdnz_falls_through, call, copy_stub, mfctr, mtctr};
use super::hypercall_ctr_decrement_tests::{bc, quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_ctr_return_copy_tests::{routine, ROUTINE};
use super::hypercall_ctr_virtual_tests::{bctr, build, sites, LOADER, NO_ENTRY, OUTSIDE};
use super::hypercall_entry_tests::{code, ordinals, pointers, word};
use super::hypercall_tests::{li, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1;

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

/// `stdu r1,imm(r1)`.
fn stdu_r1(imm: i16) -> u32 {
    word(PpuInstruction::Stdu { rs: 1, ra: 1, imm })
}

/// `lhz rt,imm(r1)`.
fn lhz(rt: u8, imm: i16) -> u32 {
    word(PpuInstruction::Lhz { rt, ra: 1, imm })
}

/// `lha rt,imm(r1)`.
fn lha(rt: u8, imm: i16) -> u32 {
    word(PpuInstruction::Lha { rt, ra: 1, imm })
}

/// `lfdu f1,imm(r1)`.
fn lfdu(imm: i16) -> u32 {
    word(PpuInstruction::Lfdu { frt: 1, ra: 1, imm })
}

/// `lfsu f1,imm(r1)`.
fn lfsu(imm: i16) -> u32 {
    word(PpuInstruction::Lfsu { frt: 1, ra: 1, imm })
}

/// `stw rs,imm(r1)`.
fn stw(rs: u8, imm: i16) -> u32 {
    word(PpuInstruction::Stw { rs, ra: 1, imm })
}

/// `addi r1,r1,imm`.
fn addi_r1(imm: i16) -> u32 {
    word(PpuInstruction::Addi { rt: 1, ra: 1, imm })
}

/// The ordinals of the two windows when a caller at [`VECTOR`], where
/// control enters in real mode, runs `bl ROUTINE; ld r5,16(r1);
/// mtctr r5; bctr`, and [`routine`] writes 0x3004 into CTR and then runs
/// `body`.
pub(super) fn after_a_routine_that_returns_a_slot(body: &[u32]) -> Vec<(u64, Option<u16>)> {
    let offset = i32::try_from(ROUTINE.wrapping_sub(VECTOR).cast_signed()).expect("in reach");
    let caller = [
        word(PpuInstruction::B {
            offset,
            aa: false,
            link: true,
        }),
        ld(5, 16),
        mtctr(5),
        word(PpuInstruction::Bcctr {
            bo: 0b10100,
            bi: 0,
            link: false,
        }),
    ];
    let segments = [
        code(VECTOR, &caller),
        window(&[]),
        quiet(),
        routine(0x3004, body),
    ];
    ordinals(0, &segments)
}

/// The ordinals of the `sc` at `nop + 4` and of the quiet window.
///
/// The image holds `nop; sc` at `nop`, and a path at [`LOADER`] that
/// loads 75 and branches to `nop`, so that `sc` resolves unless a `bctr`
/// enters `nop`. A stub at [`OUTSIDE`] builds `KERNEL | low` in CTR,
/// stores a copy of CTR at 16(r1), runs `between`, moves r0 into CTR,
/// and branches there. A code pointer names the stub.
pub(super) fn after_a_load_of_part_of_a_copy(
    nop: u16,
    low: u16,
    between: &[u32],
) -> Vec<(u64, Option<u16>)> {
    let to_nop = word(PpuInstruction::B {
        offset: i32::from(nop),
        aa: true,
        link: false,
    });
    let mut stub = build(low).to_vec();
    stub.extend([mtctr(3), mfctr(0), std(0, 16)]);
    stub.extend_from_slice(between);
    stub.extend([mtctr(0), bctr()]);
    let segments = [
        code(u64::from(nop), &[NOP, PPC_SC_1]),
        code(LOADER, &[li(11, 75), to_nop]),
        code(OUTSIDE, &stub),
        quiet(),
        pointers(&[OUTSIDE]),
    ];
    ordinals(NO_ENTRY, &segments)
}

pub(super) const UNRESOLVED: [(u64, Option<u16>); 2] = [(SC, None), (QUIET_SC, None)];

pub(super) const RESOLVED: [(u64, Option<u16>); 2] = [(SC, Some(75)), (QUIET_SC, Some(77))];

/// The routine that the `bl` calls can return with any CTR, and the
/// copy in the stack slot at 16(r1) keeps that CTR.
///
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
fn a_bctr_after_a_call_and_a_copy_of_ctr_through_a_stack_slot_leaves_every_site_unresolved() {
    let between = [call(), mfctr(0), std(0, 16), ld(0, 16), mtctr(0)];
    assert_eq!(sites(vec![copy_stub(&between)]), UNRESOLVED);
}

/// The `bdnz` falls through only when CTR is 0, so the `bctr` reaches
/// address 0 through the copy in the stack slot.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
fn a_bctr_through_a_stack_slot_after_a_bdnz_falls_through_enters_address_zero() {
    assert_eq!(
        after_bdnz_falls_through(&[mfctr(0), std(0, 16), ld(0, 16), mtctr(0)]),
        [(4, None), (QUIET_SC, Some(77))]
    );
}

/// The `stdu` moves r1 down by 112, so the copy at 16(r1) is at
/// 128(r1) after it.
///
/// [PPC-Book1 p:43 s:3.3.3] `stdu` stores RS at EA = (RA) + EXTS(DS || 0b00), then places EA into RA.
#[test]
fn a_copy_of_ctr_through_a_stack_slot_across_a_new_frame_leaves_every_site_unresolved() {
    let between = [
        call(),
        mfctr(0),
        std(0, 16),
        stdu_r1(-112),
        ld(0, 128),
        mtctr(0),
    ];
    assert_eq!(sites(vec![copy_stub(&between)]), UNRESOLVED);
}

/// The `std r4` replaces each byte of the copy at 16(r1).
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
#[test]
fn a_store_over_the_stack_slot_of_a_copy_of_ctr_keeps_each_site_resolved() {
    let between = [
        call(),
        mfctr(0),
        std(0, 16),
        std(4, 16),
        ld(0, 16),
        mtctr(0),
    ];
    assert_eq!(sites(vec![copy_stub(&between)]), RESOLVED);
}

/// The doubleword at 24(r1) holds no byte of the copy at 16(r1).
///
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
fn a_load_of_another_stack_slot_keeps_each_site_resolved() {
    let between = [call(), mfctr(0), std(0, 16), ld(0, 24), mtctr(0)];
    assert_eq!(sites(vec![copy_stub(&between)]), RESOLVED);
}

/// The word at 20(r1) is the low word of the copy of a CTR of 0, so the
/// `bctr` reaches address 0.
///
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:37 s:3.3.2] `lwz` loads the word at EA into RT 32:63 and sets RT 0:31 to 0.
#[test]
fn a_word_load_of_the_low_word_of_a_copy_of_ctr_enters_address_zero() {
    assert_eq!(
        after_bdnz_falls_through(&[mfctr(0), std(0, 16), lwz(0, 20), mtctr(0)]),
        [(4, None), (QUIET_SC, Some(77))]
    );
}

/// The doubleword at 12(r1) holds 4 bytes of the copy at 16(r1) and 4
/// bytes that the scan does not know.
///
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
fn a_load_that_holds_part_of_a_copy_of_ctr_leaves_every_site_unresolved() {
    assert_eq!(
        after_bdnz_falls_through(&[mfctr(0), std(0, 16), ld(0, 12), mtctr(0)]),
        [(4, None), (QUIET_SC, None)]
    );
}

/// A routine can leave any CTR in the slot at 16(r1), so the `bctr`
/// after the call can reach any word.
///
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:25 s:2.4] `bclr` branches to LR 0:61 || 0b00.
#[test]
fn a_real_mode_bctr_of_a_ctr_a_routine_returns_in_a_stack_slot_leaves_every_site_unresolved() {
    assert_eq!(
        after_a_routine_that_returns_a_slot(&[mfctr(5), std(5, 16)]),
        UNRESOLVED
    );
}

/// The `li r5,0` leaves no copy of CTR in a GPR at the `blr`, so only
/// the slot at 16(r1) holds the copy that the routine returns.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0, and places the sum into RT.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:25 s:2.4] `bclr` branches to LR 0:61 || 0b00.
#[test]
fn a_real_mode_bctr_of_a_ctr_a_routine_returns_only_in_a_stack_slot_leaves_every_site_unresolved() {
    assert_eq!(
        after_a_routine_that_returns_a_slot(&[mfctr(5), std(5, 16), li(5, 0)]),
        UNRESOLVED
    );
}

/// A loop moves r1 down by 16 on each pass, so the copy at 16(r1) is at
/// 32(r1) after one pass and at 48(r1) after two.
///
/// [PPC-Book1 p:43 s:3.3.3] `stdu` stores RS at EA = (RA) + EXTS(DS || 0b00), then places EA into RA.
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if CR bit BI is 1.
#[test]
fn a_copy_of_ctr_through_a_stack_slot_after_a_loop_that_moves_r1_leaves_every_site_unresolved() {
    let after_the_loop = |load: u32| {
        let between = [
            call(),
            mfctr(0),
            std(0, 16),
            stdu_r1(-16),
            bc(0b01100, -4),
            load,
            mtctr(0),
        ];
        sites(vec![copy_stub(&between)])
    };
    assert_eq!(after_the_loop(ld(0, 32)), UNRESOLVED);
    assert_eq!(after_the_loop(ld(0, 48)), UNRESOLVED);
}

/// The `lfdu` moves r1 down by 16, so the copy at 16(r1) is at 32(r1)
/// after it.
///
/// [PPC-Book1 p:105 s:4.6.2] `lfdu` loads an FPR from EA = (RA) + EXTS(D), then places EA into RA.
#[test]
fn a_copy_of_ctr_through_a_stack_slot_across_an_lfdu_leaves_every_site_unresolved() {
    let between = [call(), mfctr(0), std(0, 16), lfdu(-16), ld(0, 32), mtctr(0)];
    assert_eq!(sites(vec![copy_stub(&between)]), UNRESOLVED);
}

/// The `lfsu` moves r1 down by 16, so the copy at 16(r1) is at 32(r1)
/// after it.
///
/// [PPC-Book1 p:104 s:4.6.2] `lfsu` loads an FPR from EA = (RA) + EXTS(D), then places EA into RA.
#[test]
fn a_copy_of_ctr_through_a_stack_slot_across_an_lfsu_leaves_every_site_unresolved() {
    let between = [call(), mfctr(0), std(0, 16), lfsu(-16), ld(0, 32), mtctr(0)];
    assert_eq!(sites(vec![copy_stub(&between)]), UNRESOLVED);
}

/// The `addi` moves r1 down by 16, so the copy at 16(r1) is at 32(r1)
/// after it.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0, and places the sum into RT.
#[test]
fn a_copy_of_ctr_through_a_stack_slot_across_an_addi_of_r1_leaves_every_site_unresolved() {
    let between = [
        call(),
        mfctr(0),
        std(0, 16),
        addi_r1(-16),
        ld(0, 32),
        mtctr(0),
    ];
    assert_eq!(sites(vec![copy_stub(&between)]), UNRESOLVED);
}

/// The `stw r4` replaces 4 of the 8 bytes of the copy of a CTR of 0 at
/// 16(r1), so the doubleword there holds 4 bytes that the scan does not
/// know.
///
/// [PPC-Book1 p:42 s:3.3.3] `stw` stores RS 32:63 into the word at EA.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
#[test]
fn a_store_over_part_of_the_stack_slot_of_a_copy_of_ctr_leaves_every_site_unresolved() {
    assert_eq!(
        after_bdnz_falls_through(&[mfctr(0), std(0, 16), stw(4, 20), ld(0, 16), mtctr(0)]),
        [(4, None), (QUIET_SC, None)]
    );
}

/// CTR holds 0x8000000000000000, so its low word at 20(r1) is 0 and the
/// `bctr` reaches address 0.
///
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:37 s:3.3.2] `lwz` loads the word at EA into RT 32:63 and sets RT 0:31 to 0.
#[test]
fn a_word_load_of_the_low_word_of_a_built_ctr_through_a_stack_slot_enters_that_word() {
    assert_eq!(
        after_a_load_of_part_of_a_copy(0, 0, &[lwz(0, 20)]),
        [(4, None), (QUIET_SC, Some(77))]
    );
}

/// The `std r4` at 8(r1) writes the doubleword below the copy at 16(r1)
/// and no byte of it, so the low word at 20(r1) is still 0 and the
/// `bctr` reaches address 0.
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:37 s:3.3.2] `lwz` loads the word at EA into RT 32:63 and sets RT 0:31 to 0.
#[test]
fn a_store_just_below_the_stack_slot_of_a_built_ctr_keeps_its_low_word_entering_that_word() {
    assert_eq!(
        after_a_load_of_part_of_a_copy(0, 0, &[std(4, 8), lwz(0, 20)]),
        [(4, None), (QUIET_SC, Some(77))]
    );
}

/// CTR holds 0x8000000000000000, so its high word at 16(r1) is
/// 0x80000000, which no segment holds.
///
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:37 s:3.3.2] `lwz` loads the word at EA into RT 32:63 and sets RT 0:31 to 0.
#[test]
fn a_word_load_of_the_high_word_of_a_built_ctr_through_a_stack_slot_keeps_the_site_resolved() {
    assert_eq!(
        after_a_load_of_part_of_a_copy(0, 0, &[lwz(0, 16)]),
        [(4, Some(75)), (QUIET_SC, Some(77))]
    );
}

/// CTR holds 0x8000000000008000, so its low halfword at 22(r1) is
/// 0x8000 and the `bctr` reaches address 0x8000.
///
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:35 s:3.3.2] `lhz` loads the halfword at EA into RT 48:63 and sets RT 0:47 to 0.
#[test]
fn a_halfword_load_of_part_of_a_copy_of_ctr_enters_that_word() {
    assert_eq!(
        after_a_load_of_part_of_a_copy(0x8000, 0x8000, &[lhz(0, 22)]),
        [(0x8004, None), (QUIET_SC, Some(77))]
    );
}

/// CTR holds 0x8000000000008000, and `lha` extends the sign of its low
/// halfword, so the `bctr` reaches 0xFFFFFFFFFFFF8000, which no segment
/// holds.
///
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:36 s:3.3.2] `lha` loads the halfword at EA into RT 48:63 and fills RT 0:47 with its bit 0.
#[test]
fn a_halfword_algebraic_load_of_part_of_a_copy_of_ctr_extends_its_sign() {
    assert_eq!(
        after_a_load_of_part_of_a_copy(0x8000, 0x8000, &[lha(0, 22)]),
        [(0x8004, Some(75)), (QUIET_SC, Some(77))]
    );
}
