use super::hypercall_ctr_decrement_tests::{quiet, stub, window, QUIET_SC, SC, VECTOR};
use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_tests::KERNEL;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The kernel address of a routine that writes 0x4004, the real address
/// of the second window's `sc`, into CTR before it returns.
const ROUTINE: u64 = KERNEL + 0x6000;

/// `lis r4,0; ori r4,r4,0x4004; mtctr r4; blr` at [`ROUTINE`].
fn routine() -> Segment {
    let words = [
        word(PpuInstruction::Addis {
            rt: 4,
            ra: 0,
            imm: 0,
        }),
        word(PpuInstruction::Ori {
            ra: 4,
            rs: 4,
            imm: 0x4004,
        }),
        word(PpuInstruction::Mtctr { rs: 4 }),
        PPC_BLR,
    ];
    code(ROUTINE, &words)
}

/// The address of the word after the CTR write of a stub at [`VECTOR`].
const AFTER_WRITE: u64 = VECTOR + 12;

/// The relative branch displacement from `from` to `to`.
fn displacement(from: u64, to: u64) -> i64 {
    to.wrapping_sub(from).cast_signed()
}

/// The ordinals of the two windows when a real-mode stub at [`VECTOR`]
/// writes the address of [`ROUTINE`] into CTR, then runs `call`, then a
/// `bctr`.
fn after_call(call: u32) -> Vec<(u64, Option<u16>)> {
    let segments = [
        stub(VECTOR, ROUTINE as u16, &[call]),
        window(&[]),
        quiet(),
        routine(),
    ];
    ordinals(0, &segments)
}

/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
#[test]
fn a_real_mode_bctr_after_a_bl_leaves_every_site_unresolved() {
    let offset = i32::try_from(displacement(AFTER_WRITE, ROUTINE)).expect("in reach");
    let bl = word(PpuInstruction::B {
        offset,
        aa: false,
        link: true,
    });
    assert_eq!(after_call(bl), [(SC, None), (QUIET_SC, None)]);
}

/// [PPC-Book1 p:20 s:2.4.1] BO 1z1zz branches always.
#[test]
fn a_real_mode_bctr_after_a_bcl_that_keeps_ctr_leaves_every_site_unresolved() {
    let offset = i16::try_from(displacement(AFTER_WRITE, ROUTINE)).expect("in reach");
    let bcl = word(PpuInstruction::Bc {
        bo: 0b10100,
        bi: 0,
        offset,
        aa: false,
        link: true,
    });
    assert_eq!(after_call(bcl), [(SC, None), (QUIET_SC, None)]);
}

/// [PPC-Book1 p:20 s:2.4.1] BO 1z1zz branches always.
#[test]
fn a_real_mode_bctr_after_a_bctrl_leaves_every_site_unresolved() {
    let bctrl = word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: true,
    });
    assert_eq!(after_call(bctrl), [(SC, None), (QUIET_SC, None)]);
}

/// [PPC-Book1 p:20 s:2.4.1] BO 1z1zz branches always.
#[test]
fn a_real_mode_bctr_after_a_bclrl_leaves_every_site_unresolved() {
    let bclrl = word(PpuInstruction::Bclr {
        bo: 0b10100,
        bi: 0,
        link: true,
    });
    assert_eq!(after_call(bclrl), [(SC, None), (QUIET_SC, None)]);
}

/// The path writes no CTR, so the `bctr` reaches the CTR that
/// [`ROUTINE`] leaves.
#[test]
fn a_real_mode_bctr_after_a_call_with_no_constant_in_ctr_leaves_every_site_unresolved() {
    let offset = i32::try_from(displacement(VECTOR, ROUTINE)).expect("in reach");
    let bl = word(PpuInstruction::B {
        offset,
        aa: false,
        link: true,
    });
    let bctr = word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    });
    let segments = [code(VECTOR, &[bl, bctr]), window(&[]), quiet(), routine()];
    assert_eq!(ordinals(0, &segments), [(SC, None), (QUIET_SC, None)]);
}

/// The `bl` reaches the next word and runs no routine, so CTR at the
/// `bctr` is the built address of [`ROUTINE`]. The `bl` also sets LR to
/// the `bctr`, so the `blr` of [`ROUTINE`] runs the `bctr` again with
/// CTR at 0x4004, and the second window's `sc` runs without its
/// `li r11`.
///
/// [PPC-Book1 p:25 s:2.4] `bclr` branches to LR 0:61 || 0b00.
#[test]
fn a_real_mode_bctr_after_a_bl_to_the_next_word_reaches_the_built_address() {
    let bl = word(PpuInstruction::B {
        offset: 4,
        aa: false,
        link: true,
    });
    assert_eq!(after_call(bl), [(SC, Some(75)), (QUIET_SC, None)]);
}

/// The `bcl 20,31,$+4` reaches the next word and runs no routine, so CTR
/// at the `bctr` is the built address of [`ROUTINE`]. The `bcl` also sets
/// LR to the `bctr`, so the `blr` of [`ROUTINE`] runs the `bctr` again
/// with CTR at 0x4004, and the second window's `sc` runs without its
/// `li r11`.
///
/// [PPC-Book1 p:24 s:2.4] A taken `bc` sets NIA to its target, and a `bc` sets LR whether or not it branches.
/// [PPC-Book1 p:25 s:2.4] `bclr` branches to LR 0:61 || 0b00.
#[test]
fn a_real_mode_bctr_after_a_bcl_to_the_next_word_reaches_the_built_address() {
    let bcl = word(PpuInstruction::Bc {
        bo: 0b10100,
        bi: 31,
        offset: 4,
        aa: false,
        link: true,
    });
    assert_eq!(after_call(bcl), [(SC, Some(75)), (QUIET_SC, None)]);
}
