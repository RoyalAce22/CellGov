use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};

/// A word in the interrupt vectors, where control enters in real mode.
///
/// [PPC-Book3 p:62 s:5.4] An interrupt sets MSR bits IR and DR to 0.
pub(super) const VECTOR: u64 = KERNEL + 0x100;

/// A word outside the interrupt vectors.
const OUTSIDE: u64 = KERNEL + 0x5000;

/// The kernel address of the window `li r11,75; nop; sc; blr`.
const WINDOW: u64 = KERNEL + 0x3000;

/// The window's `sc`.
pub(super) const SC: u64 = WINDOW + 8;

/// The kernel address of a second window, `li r11,77; sc`, which no
/// branch enters.
const QUIET: u64 = KERNEL + 0x4000;

/// The second window's `sc`.
pub(super) const QUIET_SC: u64 = QUIET + 4;

/// The window, then `tail` after its `blr`.
pub(super) fn window(tail: &[u32]) -> Segment {
    let mut words = vec![li(11, 75), NOP, PPC_SC_1, PPC_BLR];
    words.extend_from_slice(tail);
    code(WINDOW, &words)
}

pub(super) fn quiet() -> Segment {
    code(QUIET, &[li(11, 77), PPC_SC_1])
}

/// `li r11,76; ba 0x3004`: a path that enters the window after its
/// `li r11` when it runs in real mode.
pub(super) fn into_window() -> [u32; 2] {
    let ba = word(PpuInstruction::B {
        offset: 0x3004,
        aa: true,
        link: false,
    });
    [li(11, 76), ba]
}

/// `lis r3,0; ori r3,r3,address; mtctr r3`.
fn ctr(address: u16) -> [u32; 3] {
    [
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
    ]
}

pub(super) fn bc(bo: u8, offset: i16) -> u32 {
    word(PpuInstruction::Bc {
        bo,
        bi: 0,
        offset,
        aa: false,
        link: false,
    })
}

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// A stub at `at`: [`ctr`] of `address`, then `between`, then `bctr`.
pub(super) fn stub(at: u64, address: u16, between: &[u32]) -> Segment {
    let mut words = ctr(address).to_vec();
    words.extend_from_slice(between);
    words.push(bctr());
    code(at, &words)
}

/// The stub reaches real address 0x3004, the window's `nop`, but no
/// constant names it.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_real_mode_bctr_after_a_bdnz_to_the_next_word_leaves_every_site_unresolved() {
    let segments = [
        stub(VECTOR, 0x3005, &[bc(0b10000, 4)]),
        window(&[]),
        quiet(),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None), (QUIET_SC, None)]);
}

/// The `bdnz` skips the `nop` to the `bctr` while the decremented CTR is
/// not 0, so the `bctr` also reaches real address 0x3004, the window's
/// `nop`.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_real_mode_bctr_that_a_bdnz_skips_to_leaves_every_site_unresolved() {
    let segments = [
        stub(VECTOR, 0x3005, &[bc(0b10000, 8), NOP]),
        window(&[]),
        quiet(),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None), (QUIET_SC, None)]);
}

/// The stub runs from the ELF entry across a 4 GB boundary, so the
/// `bdnz` target has low-order 32 bits below those of the `bdnz`, but is
/// still the `bctr` after it.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_bdnz_that_skips_to_a_bctr_across_a_4_gb_boundary_leaves_every_site_unresolved() {
    let entry = KERNEL + 0xFFFF_FFF0;
    let segments = [
        stub(entry, 0x3005, &[bc(0b10000, 8), NOP]),
        window(&[]),
        quiet(),
    ];
    assert_eq!(ordinals(entry, &segments), [(SC, None), (QUIET_SC, None)]);
}

/// The loop falls through with CTR at 0, so the `bctr` reaches the
/// kernel word at real address 0. CTR is 0x5000 after the first pass,
/// and no word is at that real address.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
#[test]
fn a_real_mode_bctr_after_a_bdnz_loop_reaches_address_zero() {
    let segments = [
        code(KERNEL, &into_window()),
        stub(VECTOR, 0x5001, &[NOP, bc(0b10000, -4)]),
        window(&[]),
        quiet(),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None), (QUIET_SC, Some(77))]);
}

/// The `bdnzl` loops back, but it calls a routine that can return to the
/// `bctr` with any CTR.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
#[test]
fn a_real_mode_bctr_after_a_linking_bdnz_leaves_every_site_unresolved() {
    let bdnzl = word(PpuInstruction::Bc {
        bo: 0b10000,
        bi: 0,
        offset: -4,
        aa: false,
        link: true,
    });
    let segments = [
        code(KERNEL, &into_window()),
        stub(VECTOR, 0x5001, &[NOP, bdnzl]),
        window(&[]),
        quiet(),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None), (QUIET_SC, None)]);
}

/// The `bdz` branches off the stub, so the `bctr` runs only when the
/// `bdz` falls through with the decremented CTR not 0, a value that the
/// scan cannot prove.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a01t decrements CTR, then branches if the decremented CTR M:63 is 0.
#[test]
fn a_real_mode_bctr_after_a_bdz_that_falls_through_leaves_every_site_unresolved() {
    let segments = [
        stub(VECTOR, 0x3010, &[bc(0b10010, 8)]),
        window(&into_window()),
        quiet(),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None), (QUIET_SC, None)]);
}

/// [PPC-Book1 p:20 s:2.4.1] BO 001at branches if the CR bit is 0 and does not decrement CTR.
#[test]
fn a_real_mode_bctr_after_a_branch_that_keeps_ctr_reaches_the_built_address() {
    let segments = [
        stub(VECTOR, 0x3010, &[bc(0b00100, 4)]),
        window(&into_window()),
        quiet(),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None), (QUIET_SC, Some(77))]);
}

/// A `bdnz` in a loop can take CTR to any word in either mode.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_bctr_after_a_bdnz_outside_real_mode_leaves_every_site_unresolved() {
    let segments = [
        stub(OUTSIDE, 0x3005, &[bc(0b10000, 4)]),
        window(&[]),
        quiet(),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None), (QUIET_SC, None)]);
}
