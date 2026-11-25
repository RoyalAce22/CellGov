use super::hypercall_ctr_copy_tests::{mfctr, mtctr};
use super::hypercall_ctr_decrement_tests::{bc, quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_ctr_virtual_tests::bctr;
use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_skip_return_tests::mflr;
use super::hypercall_tests::{li, KERNEL};
use super::*;

/// The kernel address of a routine that returns through CTR.
const ROUTINE: u64 = KERNEL + 0x7000;

const UNRESOLVED: [(u64, Option<u16>); 2] = [(SC, None), (QUIET_SC, None)];

/// The first window stays unresolved because the routine's 0x3005 names
/// the real address of its `nop`. The second window keeps its number.
const SECOND_RESOLVED: [(u64, Option<u16>); 2] = [(SC, None), (QUIET_SC, Some(77))];

/// `lis r3,0; ori r3,r3,0x3005; mtctr r3; bdnz $+4`, then `body`, then
/// `mflr r0; mtctr r0; bctr`, at [`ROUTINE`].
///
/// The `bdnz` leaves CTR at 0x3004, the real address of the window's
/// `nop`, and no constant names that value.
fn routine(body: &[u32]) -> Segment {
    let mut words = vec![
        word(PpuInstruction::Addis {
            rt: 3,
            ra: 0,
            imm: 0,
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: 0x3005,
        }),
        mtctr(3),
        bc(0b10000, 4),
    ];
    words.extend_from_slice(body);
    words.extend([mflr(0), mtctr(0), bctr()]);
    code(ROUTINE, &words)
}

/// `std rs,16(r1)`.
fn std_16(rs: u8) -> u32 {
    word(PpuInstruction::Std { rs, ra: 1, imm: 16 })
}

/// `ld rt,16(r1)`.
fn ld_16(rt: u8) -> u32 {
    word(PpuInstruction::Ld { rt, ra: 1, imm: 16 })
}

/// The ordinals of the two windows when a caller at [`VECTOR`], where
/// control enters in real mode, runs `bl ROUTINE`, then `before`, then
/// `mtctr r5; bctr`, and [`routine`] runs `body`.
fn after_a_call(before: &[u32], body: &[u32]) -> Vec<(u64, Option<u16>)> {
    let offset = i32::try_from(ROUTINE.wrapping_sub(VECTOR).cast_signed()).expect("in reach");
    let mut caller = vec![word(PpuInstruction::B {
        offset,
        aa: false,
        link: true,
    })];
    caller.extend_from_slice(before);
    caller.extend([mtctr(5), bctr()]);
    let segments = [code(VECTOR, &caller), window(&[]), quiet(), routine(body)];
    ordinals(0, &segments)
}

/// The routine copies the decremented CTR into r5, then returns through
/// CTR to the word after the `bl`. The `bctr` there reaches real address
/// 0x3004.
///
/// [PPC-Book1 p:24 s:2.4] A `bc` with BO bit 2 at 0 decrements CTR.
/// [PPC-Book1 p:82 s:3.3.13] `mflr` and `mfctr`, forms of `mfspr`, copy LR or CTR into RT.
/// [PPC-Book1 p:81 s:3.3.13] `mtctr`, a form of `mtspr`, copies RS into CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` branches to CTR 0:61 || 0b00.
#[test]
fn a_real_mode_bctr_of_a_ctr_a_routine_returns_in_a_gpr_through_bctr_leaves_every_site_unresolved()
{
    assert_eq!(after_a_call(&[], &[mfctr(5)]), UNRESOLVED);
}

/// The routine stores the copy of the decremented CTR at 16(r1) and
/// clears r5 before it returns through CTR. The caller loads the copy
/// back into r5.
///
/// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` branches to CTR 0:61 || 0b00.
#[test]
fn a_real_mode_bctr_of_a_ctr_a_routine_returns_in_a_stack_slot_through_bctr_leaves_every_site_unresolved(
) {
    let body = [mfctr(5), std_16(5), li(5, 0)];
    assert_eq!(after_a_call(&[ld_16(5)], &body), UNRESOLVED);
}

/// No copy of CTR is in r5 at the routine's `bctr`, so the `mtctr r5`
/// after the call copies no CTR that the routine wrote. A copy that
/// stays in r6 marks r6 alone.
///
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0, and places the sum into RT.
#[test]
fn a_copy_of_ctr_that_leaves_r5_before_a_return_through_bctr_keeps_the_second_window_resolved() {
    assert_eq!(after_a_call(&[], &[mfctr(5), li(5, 0)]), SECOND_RESOLVED);
    assert_eq!(after_a_call(&[], &[mfctr(6), li(6, 0)]), SECOND_RESOLVED);
    assert_eq!(after_a_call(&[], &[mfctr(6)]), SECOND_RESOLVED);
}

/// A caller at [`VECTOR`] runs `bl $+4`, which sets LR to the next word
/// and runs no routine. A `beq` there branches to [`routine`], which
/// copies the decremented CTR into r5 and returns through CTR to that
/// word. The `beq` then falls through to `mtctr r5; bctr`.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:25 s:2.4] `bcctr` branches to CTR 0:61 || 0b00.
#[test]
fn a_return_through_bctr_to_the_word_after_a_bl_to_the_next_word_brings_its_copies() {
    let to_routine =
        i16::try_from(ROUTINE.wrapping_sub(VECTOR + 4).cast_signed()).expect("in reach");
    let caller = [
        word(PpuInstruction::B {
            offset: 4,
            aa: false,
            link: true,
        }),
        bc(0b01100, to_routine),
        mtctr(5),
        bctr(),
    ];
    let with = |body: &[u32]| {
        let segments = [code(VECTOR, &caller), window(&[]), quiet(), routine(body)];
        ordinals(0, &segments)
    };
    assert_eq!(with(&[mfctr(5)]), UNRESOLVED);
    assert_eq!(with(&[mfctr(5), li(5, 0)]), SECOND_RESOLVED);
}
