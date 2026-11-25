use super::hypercall_constant_join_tests::{addi, bctr, build, mtctr, NOWHERE, WINDOW_RESOLVED};
use super::hypercall_ctr_decrement_tests::{quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_entry_tests::{code, ordinals, word};
use super::hypercall_tests::KERNEL;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The first word of the routine that the caller calls, 0x7000 past the
/// kernel base. No constant of the cases names it.
const ROUTINE: u64 = KERNEL + 0x7000;

/// The real address of the window's `li r11`.
const WINDOW_REAL: u16 = 0x3000;

/// The ordinals when a path enters the window's `nop` with r11 unset.
const WINDOW_UNRESOLVED: [(u64, Option<u16>); 2] = [(SC, None), (QUIET_SC, Some(77))];

/// `bl` from the word at `from` to the word at `to`.
///
/// [PPC-Book1 p:24 s:2.4] `b` reaches CIA + EXTS(displacement) when AA = 0, and LK = 1 sets LR to CIA + 4.
fn bl(from: u64, to: u64) -> u32 {
    let offset = i32::try_from(to.wrapping_sub(from).cast_signed()).expect("a near target");
    word(PpuInstruction::B {
        offset,
        aa: false,
        link: true,
    })
}

fn mtlr(rs: u8) -> u32 {
    word(PpuInstruction::Mtlr { rs })
}

/// The ordinals with `caller` at [`VECTOR`], `routine` at [`ROUTINE`],
/// the window and the second window.
///
/// An interrupt enters the words of the interrupt vectors in real mode.
///
/// [PPC-Book3 p:62 s:5.4] An interrupt sets MSR bits IR and DR to 0.
fn call_sites(caller: &[u32], routine: &[u32]) -> Vec<(u64, Option<u16>)> {
    let segments = [
        code(VECTOR, caller),
        code(ROUTINE, routine),
        window(&[]),
        quiet(),
    ];
    ordinals(0, &segments)
}

/// `bl` to [`ROUTINE`] at [`VECTOR`], then `tail`.
fn caller(tail: &[u32]) -> Vec<u32> {
    let mut words = vec![bl(VECTOR, ROUTINE)];
    words.extend_from_slice(tail);
    words
}

/// `addi r3,r3,step; mtctr r3; bctr`.
fn bctr_of_r3_plus(step: i16) -> [u32; 3] {
    [addi(3, 3, step), mtctr(3), bctr()]
}

/// The routine returns without a write of r3, so r3 holds the caller's
/// constant after the call, and the `addi` steps it to the window's
/// `nop`.
///
/// [PPC-Book1 p:24 s:2.4] A branch alters only CTR and LR.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
#[test]
fn a_real_mode_bctr_of_the_callers_constant_that_the_routine_keeps_then_stepped_enters_the_step() {
    let mut words = build(3, WINDOW_REAL - 4).to_vec();
    words.push(bl(VECTOR + 8, ROUTINE));
    words.extend(bctr_of_r3_plus(8));
    assert_eq!(call_sites(&words, &[PPC_BLR]), WINDOW_UNRESOLVED);
}

/// The routine builds the window's real address in r3, but its `blr`
/// goes to the LR that it wrote, so it does not return to the caller.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` copies RS into LR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_bctr_after_a_routine_whose_blr_goes_to_a_built_lr_keeps_the_sites_resolved() {
    let mut routine = build(4, NOWHERE).to_vec();
    routine.push(mtlr(4));
    routine.extend(build(3, WINDOW_REAL));
    routine.push(PPC_BLR);
    let words = caller(&bctr_of_r3_plus(4));
    assert_eq!(call_sites(&words, &routine), WINDOW_RESOLVED);
}
