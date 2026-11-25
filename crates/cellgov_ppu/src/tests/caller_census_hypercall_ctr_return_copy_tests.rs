use super::hypercall_ctr_decrement_tests::{bc, quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_tests::{li, KERNEL};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The kernel address of a routine that leaves a copy of CTR in a GPR.
pub(super) const ROUTINE: u64 = KERNEL + 0x7000;

/// `bl ROUTINE; mtctr r5; bctr` at [`VECTOR`], where control enters in
/// real mode.
fn caller() -> Segment {
    let offset = i32::try_from(ROUTINE.wrapping_sub(VECTOR).cast_signed()).expect("in reach");
    let words = [
        word(PpuInstruction::B {
            offset,
            aa: false,
            link: true,
        }),
        word(PpuInstruction::Mtctr { rs: 5 }),
        word(PpuInstruction::Bcctr {
            bo: 0b10100,
            bi: 0,
            link: false,
        }),
    ];
    code(VECTOR, &words)
}

/// `lis r3,0; ori r3,r3,address; mtctr r3`, then `body`, then `blr`, at
/// [`ROUTINE`].
pub(super) fn routine(address: u16, body: &[u32]) -> Segment {
    let mut words = vec![
        word(PpuInstruction::Addis {
            rt: 3,
            ra: 0,
            imm: 0,
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: address,
        }),
        word(PpuInstruction::Mtctr { rs: 3 }),
    ];
    words.extend_from_slice(body);
    words.push(PPC_BLR);
    code(ROUTINE, &words)
}

fn mfctr(rt: u8) -> u32 {
    word(PpuInstruction::Mfctr { rt })
}

/// The ordinals of the two windows when [`caller`] runs the routine
/// that [`routine`] builds from `address` and `body`.
fn after_routine(address: u16, body: &[u32]) -> Vec<(u64, Option<u16>)> {
    let segments = [caller(), window(&[]), quiet(), routine(address, body)];
    ordinals(0, &segments)
}

/// The routine takes 1 from CTR and returns the result in r5. The
/// `bctr` then reaches real address 0x3004, the window's `nop`, but no
/// constant names it.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
/// [PPC-Book1 p:25 s:2.4] `bclr` branches to LR 0:61 || 0b00.
#[test]
fn a_real_mode_bctr_of_a_decremented_ctr_a_routine_returns_in_a_gpr_leaves_every_site_unresolved() {
    assert_eq!(
        after_routine(0x3005, &[bc(0b10000, 4), mfctr(5)]),
        [(SC, None), (QUIET_SC, None)]
    );
}

/// The routine returns its CTR of 0x3004 in r5, so the `bctr` reaches
/// real address 0x3004, the window's `nop`.
///
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
/// [PPC-Book1 p:25 s:2.4] `bclr` branches to LR 0:61 || 0b00.
#[test]
fn a_real_mode_bctr_of_a_ctr_a_routine_returns_in_a_gpr_leaves_every_site_unresolved() {
    assert_eq!(
        after_routine(0x3004, &[mfctr(5)]),
        [(SC, None), (QUIET_SC, None)]
    );
}

/// The routine's copy of CTR does not reach its `blr` in r5, so the
/// `mtctr r5` after the call copies no CTR that the routine wrote. The
/// routine's constant 0x3005 names the window's `nop` at its real
/// address, so the first window's site stays unresolved.
///
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_copy_of_ctr_that_leaves_r5_before_the_return_keeps_the_second_window_resolved() {
    let resolved = [(SC, None), (QUIET_SC, Some(77))];
    let overwritten = [bc(0b10000, 4), mfctr(5), li(5, 0)];
    assert_eq!(after_routine(0x3005, &overwritten), resolved);
    let other_gpr = [bc(0b10000, 4), mfctr(6)];
    assert_eq!(after_routine(0x3005, &other_gpr), resolved);
}
