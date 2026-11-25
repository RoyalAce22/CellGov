use super::hypercall_ctr_decrement_tests::{
    bc, into_window, quiet, stub, window, QUIET_SC, SC, VECTOR,
};
use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_tests::KERNEL;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The real address of the path past the window's `blr`, which loads 76
/// and branches to the real address of the window's `nop`.
const PAST: u16 = 0x3010;

/// A real address with no word of the code at it.
const NOWHERE: u16 = 0x5000;

/// The ordinals of the two windows, with the path past the window's
/// `blr` and `stub` as more segments.
fn sites(stub: Segment) -> Vec<(u64, Option<u16>)> {
    ordinals(0, &[stub, window(&into_window()), quiet()])
}

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// `b` with the displacement `offset`.
fn b(offset: i32) -> u32 {
    word(PpuInstruction::B {
        offset,
        aa: false,
        link: false,
    })
}

/// `lis r4,0; ori r4,r4,address; mtctr r4`.
fn ctr_r4(address: u16) -> [u32; 3] {
    [
        word(PpuInstruction::Addis {
            rt: 4,
            ra: 0,
            imm: 0,
        }),
        word(PpuInstruction::Ori {
            ra: 4,
            rs: 4,
            imm: address,
        }),
        word(PpuInstruction::Mtctr { rs: 4 }),
    ]
}

/// The `blt` skips the second CTR write, so the `bctr` also reaches the
/// real address that the first write builds.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
#[test]
fn a_real_mode_bctr_that_a_forward_branch_reaches_past_a_ctr_write_reaches_the_first_ctr() {
    let mut between = vec![bc(0b01100, 16)];
    between.extend(ctr_r4(NOWHERE));
    assert_eq!(
        sites(stub(VECTOR, PAST, &between)),
        [(SC, None), (QUIET_SC, Some(77))]
    );
}

/// The `blt` leaves the loop with CTR at the built address less the
/// passes that the loop ran, which the scan cannot prove.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_real_mode_bctr_after_an_early_exit_from_a_bdnz_loop_leaves_every_site_unresolved() {
    let segments = stub(VECTOR, NOWHERE, &[bc(0b01100, 8), bc(0b10000, -4)]);
    assert_eq!(sites(segments), [(SC, None), (QUIET_SC, None)]);
}

/// The `b` loops back to the `blt`, and neither word writes CTR, so the
/// `blt` leaves the loop with CTR at the built address.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
#[test]
fn a_real_mode_bctr_after_a_loop_that_keeps_ctr_reaches_the_built_address() {
    let segments = stub(VECTOR, PAST, &[bc(0b01100, 8), b(-4)]);
    assert_eq!(sites(segments), [(SC, None), (QUIET_SC, Some(77))]);
}

/// The CTR of the loop is a value that the code loads, which the bound
/// takes to hold a named address. The loop falls through with CTR at 0.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_real_mode_bctr_after_an_early_exit_from_a_loop_with_a_loaded_count_adds_no_path() {
    let words = [
        word(PpuInstruction::Ld {
            rt: 3,
            ra: 1,
            imm: 0,
        }),
        word(PpuInstruction::Mtctr { rs: 3 }),
        bc(0b01100, 8),
        bc(0b10000, -4),
        bctr(),
    ];
    assert_eq!(
        sites(code(VECTOR, &words)),
        [(SC, Some(75)), (QUIET_SC, Some(77))]
    );
}

/// The vector word branches past the `bctr` to a CTR write, which
/// branches back to the `bctr`.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
#[test]
fn a_real_mode_bctr_that_a_later_branch_reaches_reaches_the_ctr_of_that_branch() {
    let mut words = vec![b(8), bctr()];
    words.extend(ctr_r4(PAST));
    words.push(b(-16));
    assert_eq!(
        sites(code(VECTOR, &words)),
        [(SC, None), (QUIET_SC, Some(77))]
    );
}

/// The kernel address of a `blr` that no path to a `bctr` reaches.
const FAR: u64 = KERNEL + 0x6000;

/// The `bdnz` branches off the path, so the `bctr` runs only when the
/// `bdnz` falls through with CTR at 0. With no word at real address 0,
/// the `bctr` enters no window. With [`into_window`] at real address 0,
/// the `bctr` enters the window after its `li r11`.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
#[test]
fn a_real_mode_bctr_after_a_bdnz_to_a_later_word_off_the_path_reaches_address_zero() {
    let offset = i16::try_from(FAR - (VECTOR + 12)).expect("a near branch");
    let scanned = |at_zero: Option<Segment>| {
        let mut segments: Vec<Segment> = at_zero.into_iter().collect();
        segments.extend([
            stub(VECTOR, PAST, &[bc(0b10000, offset)]),
            window(&into_window()),
            quiet(),
            code(FAR, &[PPC_BLR]),
        ]);
        ordinals(0, &segments)
    };
    assert_eq!(scanned(None), [(SC, Some(75)), (QUIET_SC, Some(77))]);
    assert_eq!(
        scanned(Some(code(KERNEL, &into_window()))),
        [(SC, None), (QUIET_SC, Some(77))]
    );
}
