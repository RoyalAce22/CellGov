use super::hypercall_constant_join_tests::skip;
use super::hypercall_ctr_decrement_tests::SC;
use super::hypercall_entry_tests::word;
use super::hypercall_skip_return_tests::{mflr, mtlr, ordinals_with_routine};
use super::hypercall_tests::{li, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The routine adds 3 to a copy of LR with `addi`, then adds 1 more with
/// `add`, and returns there, which reaches the `lis`. The scan follows no
/// constant through `add`, so the lost mark of the first step must hold
/// through the second step.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0.
/// [PPC-Book1 p:52 s:3.3.8] `add` places the sum of RA and RB into RT.
#[test]
fn a_return_to_a_copy_of_lr_stepped_twice_leaves_every_site_unresolved() {
    let add_3 = word(PpuInstruction::Addi {
        rt: 11,
        ra: 11,
        imm: 3,
    });
    let add_1 = word(PpuInstruction::Add {
        rt: 11,
        ra: 11,
        rb: 12,
        oe: false,
        rc: false,
    });
    let routine = [mflr(11), add_3, li(12, 1), add_1, mtlr(11), PPC_BLR];
    assert_eq!(ordinals_with_routine(&routine), [(SC, None)]);
}

/// The routine loads the word after the return point with `lwzu`, which
/// also places the address of that word into its base register, a copy of
/// LR. It returns to that address, which reaches the `lis`.
///
/// [PPC-Book1 p:33 s:3.3.2] A load with update places the EA into RA.
#[test]
fn a_return_to_a_copy_of_lr_that_a_load_with_update_steps_leaves_every_site_unresolved() {
    let load_next = word(PpuInstruction::Lwzu {
        rt: 0,
        ra: 11,
        imm: 4,
    });
    let routine = [mflr(11), load_next, mtlr(11), PPC_BLR];
    assert_eq!(ordinals_with_routine(&routine), [(SC, None)]);
}

/// `add rt,ra,rb`.
///
/// [PPC-Book1 p:52 s:3.3.8] `add` places the sum of RA and RB into RT.
fn add(rt: u8, ra: u8, rb: u8) -> u32 {
    word(PpuInstruction::Add {
        rt,
        ra,
        rb,
        oe: false,
        rc: false,
    })
}

fn addi(rt: u8, ra: u8, imm: i16) -> u32 {
    word(PpuInstruction::Addi { rt, ra, imm })
}

/// Each word computes r11 from the copy of LR in r11, alone or with r12,
/// in each operand that it reads. So each return to r11 is a return to a
/// stepped copy, also from a word that can give its source unchanged.
///
/// [PPC-Book1 p:55 s:3.3.8] `neg` places the negation of RA into RT.
/// [PPC-Book1 p:65 s:3.3.11] A logical instruction places its result into RA.
/// [PPC-Book1 p:76 s:3.3.12.1] `rlwimi` inserts the rotated RS into RA under the mask, so the result also holds bits of RA.
/// [PPC-Book1 p:33 s:3.3.2] A load with update places the EA into RA.
#[test]
fn a_return_to_a_copy_of_lr_that_each_computing_form_steps_leaves_every_site_unresolved() {
    let rlwimi = |rs| PpuInstruction::Rlwimi {
        ra: 11,
        rs,
        sh: 1,
        mb: 0,
        me: 30,
        rc: false,
    };
    let forms = [
        PpuInstruction::Neg {
            rt: 11,
            ra: 11,
            oe: false,
            rc: false,
        },
        PpuInstruction::Add {
            rt: 11,
            ra: 11,
            rb: 11,
            oe: false,
            rc: false,
        },
        PpuInstruction::And {
            ra: 11,
            rs: 11,
            rb: 12,
            rc: false,
        },
        PpuInstruction::And {
            ra: 11,
            rs: 11,
            rb: 11,
            rc: false,
        },
        PpuInstruction::Xori {
            ra: 11,
            rs: 11,
            imm: 1,
        },
        rlwimi(12),
        rlwimi(11),
        PpuInstruction::Lwzux {
            rt: 0,
            ra: 11,
            rb: 12,
        },
        PpuInstruction::Lwzux {
            rt: 0,
            ra: 11,
            rb: 11,
        },
    ];
    assert_eq!(forms.len(), 9);
    for form in forms {
        assert_eq!(crate::decode::decode(word(form)), Ok(form));
        let routine = [mflr(11), word(form), mtlr(11), PPC_BLR];
        assert_eq!(ordinals_with_routine(&routine), [(SC, None)], "{form:?}");
    }
}

/// The `li r11` writes a constant over the copy of LR in r11, so the
/// `addi` steps that constant, and the routine returns to 0x7010, which
/// names no word. The `li r6` writes a register that holds no copy.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_return_to_a_constant_written_over_a_copy_of_lr_keeps_the_site_resolved() {
    let routine = [
        mflr(11),
        li(11, 0x7000),
        li(6, 0),
        addi(5, 11, 0x10),
        mtlr(5),
        PPC_BLR,
    ];
    assert_eq!(ordinals_with_routine(&routine), [(SC, Some(75))]);
}

/// The `blt` after the `mflr` goes past the next word to the join. Each
/// case takes r11 to the join as a copy of LR, as a stepped copy, or as
/// a constant on each of the two paths, and the `add` after the join
/// computes r5 from r11. The routine returns to r5.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:52 s:3.3.8] `add` places the sum of RA and RB into RT.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_return_to_an_add_of_a_copy_of_lr_that_a_join_keeps_on_either_path_leaves_every_site_unresolved(
) {
    let tail = [li(12, 0), add(5, 11, 12), mtlr(5), PPC_BLR];
    let cases = [
        ("copy on both paths", vec![mflr(11), skip(1), NOP]),
        (
            "stepped copy on both paths",
            vec![mflr(11), addi(11, 11, 8), skip(1), addi(11, 11, 4)],
        ),
        (
            "copy on one path and stepped copy on the other",
            vec![mflr(11), skip(1), addi(11, 11, 4)],
        ),
        (
            "stepped copy on one path and constant on the other",
            vec![mflr(11), addi(11, 11, 4), skip(1), li(11, 0x7000)],
        ),
        (
            "copy on one path and constant on the other",
            vec![mflr(11), skip(1), li(11, 0x7000)],
        ),
    ];
    assert_eq!(cases.len(), 5);
    for (case, mut routine) in cases {
        routine.extend(tail);
        assert_eq!(ordinals_with_routine(&routine), [(SC, None)], "{case}");
    }
}

/// One path takes a copy of LR in r11 to the join and the other a stepped
/// copy. The `li r11` at the join writes a constant over both, so the
/// routine returns to 0x7010.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0.
#[test]
fn a_return_to_a_constant_written_at_a_join_over_a_copy_and_a_stepped_copy_keeps_the_site_resolved()
{
    let routine = [
        mflr(11),
        skip(1),
        addi(11, 11, 4),
        li(11, 0x7000),
        addi(5, 11, 0x10),
        mtlr(5),
        PPC_BLR,
    ];
    assert_eq!(ordinals_with_routine(&routine), [(SC, Some(75))]);
}

/// The routine enters with an LR that holds no constant. The `blt` takes
/// that LR to the `mflr`, and the fall-through takes a constant there. So
/// the `mflr` copies an LR that the scan does not know, and the routine
/// returns to a stepped copy of it.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:82 s:3.3.13] `mflr`, a form of `mfspr`, copies LR into RT.
#[test]
fn a_return_to_a_stepped_copy_of_an_lr_that_one_path_brings_unknown_leaves_every_site_unresolved() {
    let routine = [
        skip(2),
        li(3, 0x7000),
        mtlr(3),
        mflr(11),
        addi(11, 11, 4),
        mtlr(11),
        PPC_BLR,
    ];
    assert_eq!(ordinals_with_routine(&routine), [(SC, None)]);
}
