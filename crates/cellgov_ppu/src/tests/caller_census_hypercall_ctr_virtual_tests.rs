use super::hypercall_ctr_decrement_tests::{bc, quiet, window, QUIET_SC, SC};
use super::hypercall_entry_tests::{code, data_bytes, ordinals, word, Segment};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};

/// A word outside the interrupt vectors, where control does not run in
/// real mode.
pub(super) const OUTSIDE: u64 = KERNEL + 0x5000;

/// An ELF entry that no segment holds.
pub(super) const NO_ENTRY: u64 = KERNEL + 0x7000;

/// The kernel address of a path that loads 75 and branches to address 0.
pub(super) const LOADER: u64 = KERNEL + 0x6000;

/// `lis r3,-32768; rldicr r3,r3,32,31; ori r3,r3,low`: the kernel
/// address `KERNEL | low` in r3.
pub(super) fn build(low: u16) -> [u32; 3] {
    [
        word(PpuInstruction::Addis {
            rt: 3,
            ra: 0,
            imm: -32768,
        }),
        word(PpuInstruction::Rldicr {
            ra: 3,
            rs: 3,
            sh: 32,
            me: 31,
            rc: false,
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: low,
        }),
    ]
}

pub(super) fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// A stub at [`OUTSIDE`]: [`build`] of `low`, `mtctr r3`, then `between`,
/// then `bctr`.
fn ctr_stub(low: u16, between: &[u32]) -> Segment {
    let mut words = build(low).to_vec();
    words.push(word(PpuInstruction::Mtctr { rs: 3 }));
    words.extend_from_slice(between);
    words.push(bctr());
    code(OUTSIDE, &words)
}

/// The ordinals of the window and of the quiet window, with `more` as
/// more segments.
pub(super) fn sites(more: Vec<Segment>) -> Vec<(u64, Option<u16>)> {
    let mut segments = more;
    segments.push(window(&[]));
    segments.push(quiet());
    ordinals(NO_ENTRY, &segments)
}

/// The `bctr` reaches the window's `nop` at 0x8000000000003004, between
/// its `li r11` and `sc`.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_outside_real_mode_to_a_built_address_with_low_bits_set_enters_its_word() {
    assert_eq!(
        sites(vec![ctr_stub(0x3005, &[])]),
        [(SC, None), (QUIET_SC, Some(77))]
    );
}

/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_outside_real_mode_to_a_built_address_with_low_bits_set_enters_its_word() {
    let mut words = build(0x3007).to_vec();
    words.extend([word(PpuInstruction::Mtlr { rs: 3 }), PPC_BLR]);
    assert_eq!(
        sites(vec![code(OUTSIDE, &words)]),
        [(SC, None), (QUIET_SC, Some(77))]
    );
}

/// A doubleword of the image names 0x8000000000003006, so an indirect
/// branch to a value that the code loads can reach the window's `nop`.
///
/// [PPC-Book1 p:20 s:2.4.1] A branch ignores bits 62 and 63 of the target address that it generates.
#[test]
fn a_named_doubleword_with_low_bits_set_enters_its_word() {
    let named = data_bytes(KERNEL + 0x8000, (KERNEL | 0x3006).to_be_bytes().to_vec());
    assert_eq!(sites(vec![named]), [(SC, None), (QUIET_SC, Some(77))]);
}

/// The `bdnz` falls through only when the decremented CTR is 0, so the
/// `bctr` reaches address 0, between the `li r11` at [`LOADER`] and the
/// `sc` at 4. No address that the image names is 0.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_bctr_outside_real_mode_after_a_bdnz_falls_through_enters_address_zero() {
    let to_zero = word(PpuInstruction::B {
        offset: 0,
        aa: true,
        link: false,
    });
    let segments = [
        code(0, &[NOP, PPC_SC_1]),
        code(LOADER, &[li(11, 75), to_zero]),
        code(OUTSIDE, &[bc(0b10000, 8), bctr(), PPC_BLR]),
        quiet(),
    ];
    assert_eq!(
        ordinals(NO_ENTRY, &segments),
        [(4, None), (QUIET_SC, Some(77))]
    );
}

/// Eight `bdnz` to the next word take CTR from 0x800000000000300C to the
/// window's `nop`, but a loop can run a `bdnz` more than once, so the
/// `bctr` can reach any word.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_bctr_outside_real_mode_after_eight_bdnz_leaves_every_site_unresolved() {
    let bdnz = [bc(0b10000, 4); 8];
    assert_eq!(
        sites(vec![ctr_stub(0x300C, &bdnz)]),
        [(SC, None), (QUIET_SC, None)]
    );
}

/// The routine that the `bl` calls is not in the image, and it can return
/// with any CTR.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
#[test]
fn a_bctr_outside_real_mode_after_a_call_leaves_every_site_unresolved() {
    let call = word(PpuInstruction::B {
        offset: 0x1000,
        aa: false,
        link: true,
    });
    assert_eq!(
        sites(vec![ctr_stub(0x300C, &[call])]),
        [(SC, None), (QUIET_SC, None)]
    );
}
