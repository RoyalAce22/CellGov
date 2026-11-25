use super::hypercall_constant_join_tests::{
    bctr, build, mtctr, sites, skip, EVERY_UNRESOLVED, NOWHERE, WINDOW_RESOLVED,
};
use super::hypercall_ctr_copy_tests::mfctr;
use super::hypercall_ctr_decrement_tests::{bc, quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_entry_tests::{code, ordinals, word};
use super::hypercall_real_mode_switch_tests::{mtmsrd, mtspr};
use super::hypercall_skip_return_tests::{mflr, mtlr};
use super::hypercall_tests::KERNEL;
use super::hypercall_unfollowed_write_tests::{mfmsr, mfspr};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;
use cellgov_ps3_abi::hw::ppu_supervisor::{SPR_SPRG0, SPR_SRR0, SPR_SRR1};

/// The kernel address of the routine that a caller at [`VECTOR`] calls.
const ROUTINE: u64 = KERNEL + 0x7000;

/// `lis r3,0; ori r3,r3,0x5000; mtctr r3`.
fn ctr_of_nowhere() -> Vec<u32> {
    let mut words = build(3, NOWHERE).to_vec();
    words.push(mtctr(3));
    words
}

/// [`ctr_of_nowhere`], then `bdnz $+4`. The scan cannot prove the CTR
/// of the taken path, and no GPR holds a lost mark.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
fn lost_ctr() -> Vec<u32> {
    let mut words = ctr_of_nowhere();
    words.push(bc(0b10000, 4));
    words
}

/// [`lost_ctr`], then `mfctr r4`, then `tail`.
///
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
fn lost_copy_in_r4(tail: &[u32]) -> Vec<u32> {
    let mut words = lost_ctr();
    words.push(mfctr(4));
    words.extend_from_slice(tail);
    words
}

/// [`ctr_of_nowhere`], then `mfctr r4`, then `tail`. r4 holds a copy
/// of the constant 0x5000.
fn known_copy_in_r4(tail: &[u32]) -> Vec<u32> {
    let mut words = ctr_of_nowhere();
    words.push(mfctr(4));
    words.extend_from_slice(tail);
    words
}

/// The ordinals when a caller at [`VECTOR`] runs `bl ROUTINE`, then
/// `after`, and the routine runs [`lost_ctr`], then `body`.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
fn with_routine(after: &[u32], body: &[u32]) -> Vec<(u64, Option<u16>)> {
    let offset = i32::try_from(ROUTINE.wrapping_sub(VECTOR).cast_signed()).expect("in reach");
    let mut caller = vec![word(PpuInstruction::B {
        offset,
        aa: false,
        link: true,
    })];
    caller.extend_from_slice(after);
    let mut routine = lost_ctr();
    routine.extend_from_slice(body);
    let segments = [
        code(VECTOR, &caller),
        window(&[]),
        quiet(),
        code(ROUTINE, &routine),
    ];
    ordinals(0, &segments)
}

/// Each pair of words that moves r4 into CR or an SPR and then moves that
/// register into r5.
///
/// [PPC-Book1 p:83 s:3.3.13] `mtcrf` writes the CR fields that FXM selects from RS, and `mfcr` places CR into RT.
/// [PPC-Book1 p:124 s:5.1.1] `mtocrf` and `mfocrf` move one CR field between CR and a GPR.
/// [PPC-Book1 p:81 s:3.3.13] `mtspr` copies RS into the SPR that its SPR field names.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT.
/// [PPC-Book3 p:16 s:3.3.3] Problem-state software may read SPRG3 through a second SPR number.
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 1 places bits 48 and 62 of RS into the MSR.
/// [PPC-Book3 p:23 s:3.4] `mfmsr` places the MSR into RT.
fn round_trips() -> Vec<[u32; 2]> {
    use PpuInstruction as I;
    vec![
        [mtspr(SPR_SPRG0, 4), mfspr(5, SPR_SPRG0)],
        [mtspr(SPR_SPRG0 + 3, 4), word(I::Mfsprg3 { rt: 5 })],
        [word(I::Mtcrf { rs: 4, crm: 0xFF }), word(I::Mfcr { rt: 5 })],
        [
            word(I::Mtocrf { rs: 4, crm: 0x80 }),
            word(I::Mfocrf { rt: 5, crm: 0x80 }),
        ],
        [word(I::Mtvrsave { rs: 4 }), word(I::Mfvrsave { rt: 5 })],
        [word(I::Mtxer { rs: 4 }), word(I::Mfxer { rt: 5 })],
        [mtspr(SPR_SRR0, 4), mfspr(5, SPR_SRR0)],
        [mtspr(SPR_SRR1, 4), mfspr(5, SPR_SRR1)],
        [mtmsrd(4, true), mfmsr(5)],
    ]
}

/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_lost_ctr_copy_moved_through_cr_or_an_spr_leaves_every_site_unresolved() {
    let pairs = round_trips();
    assert_eq!(pairs.len(), 9);
    for [into, out_of] in pairs {
        let words = lost_copy_in_r4(&[into, out_of, mtctr(5), bctr()]);
        assert_eq!(
            sites(VECTOR, &words),
            EVERY_UNRESOLVED,
            "{into:08x} {out_of:08x}"
        );
    }
}

/// r3 holds the constant 0x5000 and no copy of CTR, so the move through
/// SPRG0 brings no copy to the `mtctr`.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtspr` copies RS into the SPR that its SPR field names.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_gpr_with_no_ctr_copy_moved_through_sprg0_keeps_the_sites_resolved() {
    let mut words = ctr_of_nowhere();
    words.extend([mtspr(SPR_SPRG0, 3), mfspr(5, SPR_SPRG0), mtctr(5), bctr()]);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// r4 holds a copy of the constant 0x5000. The cell of CR and the SPRs
/// keeps no constant, so the copy that comes back out of SPRG0 is one
/// that the scan cannot prove.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtspr` copies RS into the SPR that its SPR field names.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_known_ctr_copy_moved_through_sprg0_leaves_every_site_unresolved() {
    let words = known_copy_in_r4(&[mtspr(SPR_SPRG0, 4), mfspr(5, SPR_SPRG0), mtctr(5), bctr()]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// CTR holds 0x13004. The `lhz` loads the low-order halfword of its copy
/// back from the stack, and that part goes through SPRG0 into CTR. No
/// constant of the image names 0x3004, the real address of the window's
/// `nop`, so the `bctr` is lost.
///
/// [PPC-Book1 p:51 s:3.3.8] `addis` adds EXTS(SI || 0x0000) to 0 when RA is 0.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:81 s:3.3.13] `mtspr` copies RS into the SPR that its SPR field names.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_part_of_a_known_ctr_copy_moved_through_sprg0_leaves_every_site_unresolved() {
    use PpuInstruction as I;
    let words = [
        word(I::Addis {
            rt: 3,
            ra: 0,
            imm: 1,
        }),
        word(I::Ori {
            ra: 3,
            rs: 3,
            imm: 0x3004,
        }),
        mtctr(3),
        mfctr(4),
        word(I::Std {
            rs: 4,
            ra: 1,
            imm: 16,
        }),
        word(I::Lhz {
            rt: 5,
            ra: 1,
            imm: 22,
        }),
        mtspr(SPR_SPRG0, 5),
        mfspr(6, SPR_SPRG0),
        mtctr(6),
        bctr(),
    ];
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The `blt` makes the `mfspr` a join word, so the move into SPRG0 reaches
/// it only through what the join brings.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_lost_ctr_copy_moved_through_sprg0_across_a_join_leaves_every_site_unresolved() {
    let words = lost_copy_in_r4(&[
        mtspr(SPR_SPRG0, 4),
        skip(0),
        mfspr(5, SPR_SPRG0),
        mtctr(5),
        bctr(),
    ]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The routine moves a copy of its lost CTR into SPRG0 and returns. The
/// caller reads SPRG0 after the call.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtspr` copies RS into the SPR that its SPR field names.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_sprg0_that_a_routine_wrote_from_a_lost_ctr_copy_leaves_every_site_unresolved() {
    let after = [mfspr(5, SPR_SPRG0), mtctr(5), bctr()];
    let moved = [mfctr(6), mtspr(SPR_SPRG0, 6), PPC_BLR];
    assert_eq!(with_routine(&after, &moved), EVERY_UNRESOLVED);
    assert_eq!(with_routine(&after, &[mfctr(6), PPC_BLR]), WINDOW_RESOLVED);
}

/// The ordinals when a caller at [`VECTOR`] runs `before`, then
/// `bl ROUTINE`, then `blr`, and the routine runs `mfspr r5,SPRG0`,
/// `mtctr r5` and `bctr`.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT.
fn calling_a_reader_of_sprg0(before: &[u32]) -> Vec<(u64, Option<u16>)> {
    let mut caller = before.to_vec();
    let at = VECTOR + 4 * u64::try_from(caller.len()).expect("in reach");
    let offset = i32::try_from(ROUTINE.wrapping_sub(at).cast_signed()).expect("in reach");
    caller.push(word(PpuInstruction::B {
        offset,
        aa: false,
        link: true,
    }));
    caller.push(PPC_BLR);
    let routine = [mfspr(5, SPR_SPRG0), mtctr(5), bctr()];
    let segments = [
        code(VECTOR, &caller),
        window(&[]),
        quiet(),
        code(ROUTINE, &routine),
    ];
    ordinals(0, &segments)
}

/// The caller moves a copy of its lost CTR into SPRG0 before the call,
/// and the routine reads SPRG0 into CTR. With no move, SPRG0 holds no
/// copy.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtspr` copies RS into the SPR that its SPR field names.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_sprg0_in_a_routine_that_the_caller_wrote_from_a_lost_ctr_copy_leaves_every_site_unresolved(
) {
    let moved = lost_copy_in_r4(&[mtspr(SPR_SPRG0, 4)]);
    assert_eq!(calling_a_reader_of_sprg0(&moved), EVERY_UNRESOLVED);
    assert_eq!(
        calling_a_reader_of_sprg0(&lost_copy_in_r4(&[])),
        WINDOW_RESOLVED
    );
}

/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:82 s:3.3.13] `mflr` copies LR into RT.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_branch_to_a_lost_ctr_copy_moved_into_lr_leaves_every_site_unresolved() {
    let blr = lost_copy_in_r4(&[mtlr(4), PPC_BLR]);
    assert_eq!(sites(VECTOR, &blr), EVERY_UNRESOLVED);
    let bctr_of_lr = lost_copy_in_r4(&[mtlr(4), mflr(5), mtctr(5), bctr()]);
    assert_eq!(sites(VECTOR, &bctr_of_lr), EVERY_UNRESOLVED);
}

/// The `bcl` to the next word writes LR over the copy, so the `blr`
/// reaches the word after the `bcl`.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_after_a_link_over_a_lost_ctr_copy_in_lr_keeps_the_sites_resolved() {
    let bcl = word(PpuInstruction::Bc {
        bo: 0b10100,
        bi: 31,
        offset: 4,
        aa: false,
        link: true,
    });
    let words = lost_copy_in_r4(&[mtlr(4), bcl, PPC_BLR]);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// CTR holds 0x13004. The `lhz` loads the low-order halfword of its copy
/// back from the stack, so the `blr` reaches real address 0x3004, the
/// window's `nop`. No constant of the image names that value.
///
/// [PPC-Book1 p:51 s:3.3.8] `addis` adds EXTS(SI || 0x0000) to 0 when RA is 0.
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_real_mode_blr_of_a_part_of_a_known_ctr_copy_reaches_that_part() {
    use PpuInstruction as I;
    let words = [
        word(I::Addis {
            rt: 3,
            ra: 0,
            imm: 1,
        }),
        word(I::Ori {
            ra: 3,
            rs: 3,
            imm: 0x3004,
        }),
        mtctr(3),
        mfctr(4),
        word(I::Std {
            rs: 4,
            ra: 1,
            imm: 16,
        }),
        word(I::Lhz {
            rt: 5,
            ra: 1,
            imm: 22,
        }),
        mtlr(5),
        PPC_BLR,
    ];
    assert_eq!(sites(VECTOR, &words), [(SC, None), (QUIET_SC, Some(77))]);
}

/// The routine puts a copy of its lost CTR in LR and returns through CTR
/// to the word after the call, so the caller's `blr` takes that copy.
/// With LR restored before the return, the `blr` reaches its own word.
///
/// [PPC-Book1 p:82 s:3.3.13] `mflr` copies LR into RT.
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_blr_after_a_return_through_bctr_with_a_lost_ctr_copy_in_lr_leaves_every_site_unresolved() {
    let in_lr = [mfctr(6), mflr(0), mtlr(6), mtctr(0), bctr()];
    assert_eq!(with_routine(&[PPC_BLR], &in_lr), EVERY_UNRESOLVED);
    let restored = [mfctr(6), mflr(0), mtlr(6), mtlr(0), mtctr(0), bctr()];
    assert_eq!(with_routine(&[PPC_BLR], &restored), WINDOW_RESOLVED);
}
