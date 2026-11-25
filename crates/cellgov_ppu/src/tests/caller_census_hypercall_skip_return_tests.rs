use super::hypercall_ctr_copy_tests::{call, mtctr};
use super::hypercall_ctr_decrement_tests::{window, SC, VECTOR};
use super::hypercall_ctr_virtual_tests::{bctr, OUTSIDE};
use super::hypercall_entry_tests::{code, ordinals, pointers, word, Segment};
use super::hypercall_tests::li;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The routine that [`call`] at [`OUTSIDE`] reaches.
const ROUTINE: u64 = OUTSIDE + 0x1000;

/// A code pointer names [`OUTSIDE`], which holds [`call`], then `b .`,
/// then `lis r3,0; ori r3,r3,0x3004; blr`. Only a return to the word two
/// past the call reaches the `lis`. The built 0x3004 is the real address
/// of the window's `nop`.
fn caller() -> [Segment; 2] {
    let stub = [
        call(),
        word(PpuInstruction::B {
            offset: 0,
            aa: false,
            link: false,
        }),
        word(PpuInstruction::Addis {
            rt: 3,
            ra: 0,
            imm: 0,
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: 0x3004,
        }),
        PPC_BLR,
    ];
    [code(OUTSIDE, &stub), pointers(&[OUTSIDE])]
}

/// The segments of [`caller`], the routine at [`ROUTINE`] with `routine`
/// as its words, a `blr` at the vector, and the window.
pub(super) fn ordinals_with_routine(routine: &[u32]) -> Vec<(u64, Option<u16>)> {
    let [stub, data] = caller();
    let segments = [
        code(VECTOR, &[PPC_BLR]),
        stub,
        data,
        code(ROUTINE, routine),
        window(&[]),
    ];
    ordinals(0, &segments)
}

pub(super) fn mflr(rt: u8) -> u32 {
    word(PpuInstruction::Mflr { rt })
}

pub(super) fn mtlr(rs: u8) -> u32 {
    word(PpuInstruction::Mtlr { rs })
}

/// The routine adds 4 to a copy of LR in a register and returns there. So
/// control reaches the `lis` after the `b .`, and its constant names the
/// real alias of the window's `nop`, which the `blr` at the vector can
/// enter.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
/// [PPC-Book1 p:52 s:3.3.8] `add` places the sum of RA and RB into RT.
#[test]
fn a_return_to_a_stepped_copy_of_lr_leaves_every_site_unresolved() {
    let add = word(PpuInstruction::Add {
        rt: 0,
        ra: 0,
        rb: 12,
        oe: false,
        rc: false,
    });
    let routine = [mflr(0), li(12, 4), add, mtlr(0), PPC_BLR];
    assert_eq!(ordinals_with_routine(&routine), [(SC, None)]);
}

/// The routine adds 4 to a copy of LR with `addi` and branches there
/// through CTR, which reaches the same `lis`.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_branch_to_a_stepped_copy_of_lr_through_ctr_leaves_every_site_unresolved() {
    let add_4 = word(PpuInstruction::Addi {
        rt: 12,
        ra: 12,
        imm: 4,
    });
    let routine = [mflr(12), add_4, mtctr(12), bctr()];
    assert_eq!(ordinals_with_routine(&routine), [(SC, None)]);
}

/// The routine moves a copy of LR through a second register and back to
/// LR unchanged, as a prologue and an epilogue do. So it returns to the
/// word after the call, and the `lis` stays out of reach.
///
/// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
#[test]
fn a_return_to_an_unstepped_copy_of_lr_keeps_the_site_resolved() {
    let copy = word(PpuInstruction::Or {
        ra: 12,
        rs: 0,
        rb: 0,
        rc: false,
    });
    let routine = [mflr(0), copy, mtlr(12), PPC_BLR];
    assert_eq!(ordinals_with_routine(&routine), [(SC, Some(75))]);
}
