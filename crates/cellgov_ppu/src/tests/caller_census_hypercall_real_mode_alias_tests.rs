use super::hypercall_entry_tests::{code, data_bytes, ordinals, word, Segment};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};

/// A word in the interrupt vectors, where control enters in real mode.
///
/// [PPC-Book3 p:62 s:5.4] An interrupt sets MSR bits IR and DR to 0.
const VECTOR: u64 = KERNEL + 0x100;

/// A word outside the interrupt vectors.
const OUTSIDE: u64 = KERNEL + 0x5000;

/// The address of a data segment that names one word.
const DATA: u64 = KERNEL + 0x4_0000;

/// The address of a window in a segment at a real address.
const REAL_WINDOW: u64 = 0x3000;

/// The address of a window in a segment at a real address above 4 GiB.
/// No 32-bit half of the doubleword [`ALIAS`] names a word of the code.
const HIGH_WINDOW: u64 = 0x1_0000_3000;

/// The kernel address of a word with the real address of the word
/// between the `li r11` and the `sc` of the window at [`HIGH_WINDOW`].
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
const ALIAS: u64 = KERNEL + HIGH_WINDOW + 4;

/// A data segment whose one doubleword names `address`.
fn naming(address: u64) -> Segment {
    data_bytes(DATA, address.to_be_bytes().to_vec())
}

/// `ld r3,0(r4)`: a value that the scan cannot know.
fn ld_r3() -> u32 {
    word(PpuInstruction::Ld {
        rt: 3,
        ra: 4,
        imm: 0,
    })
}

fn mtctr_r3() -> u32 {
    word(PpuInstruction::Mtctr { rs: 3 })
}

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// The site of a window at [`HIGH_WINDOW`] that loads 75, with a word
/// at [`ALIAS`] that [`naming`] names and with `stub` at `at`.
fn real_window_site(at: u64, stub: &[u32]) -> Vec<(u64, Option<u16>)> {
    let segments = [
        code(at, stub),
        code(KERNEL + HIGH_WINDOW, &[NOP, NOP, NOP]),
        naming(ALIAS),
        code(HIGH_WINDOW, &[li(11, 75), NOP, PPC_SC_1]),
    ];
    ordinals(0, &segments)
}

const REAL_SC: u64 = REAL_WINDOW + 8;

const HIGH_SC: u64 = HIGH_WINDOW + 8;

#[test]
fn a_real_mode_bctr_to_a_loaded_address_enters_the_real_alias_of_a_named_word() {
    let stub = [ld_r3(), mtctr_r3(), bctr()];
    assert_eq!(real_window_site(VECTOR, &stub), [(HIGH_SC, None)]);
    assert_eq!(real_window_site(OUTSIDE, &stub), [(HIGH_SC, Some(75))]);
}

#[test]
fn a_real_mode_blr_to_a_loaded_address_enters_the_real_alias_of_a_named_word() {
    let stub = [ld_r3(), word(PpuInstruction::Mtlr { rs: 3 }), PPC_BLR];
    assert_eq!(real_window_site(VECTOR, &stub), [(HIGH_SC, None)]);
    assert_eq!(real_window_site(OUTSIDE, &stub), [(HIGH_SC, Some(75))]);
}

/// The kernel address of a window that loads 75 and runs `sc`.
const WINDOW: u64 = KERNEL + 0x3000;

/// The kernel address of the path past the window's `blr`, which loads
/// a number and branches to the real address of the window's `nop`.
const PAST: u64 = WINDOW + 0x10;

/// The site of the window at [`WINDOW`], with a path at [`PAST`] that
/// loads `other` and that [`naming`] names, and with `stub` at `at`.
///
/// [PPC-Book1 p:24 s:2.4] `b` with AA = 1 reaches EXTS(displacement).
fn past_site(at: u64, other: i16, stub: &[u32]) -> Vec<(u64, Option<u16>)> {
    let ba = word(PpuInstruction::B {
        offset: 0x3004,
        aa: true,
        link: false,
    });
    let segments = [
        code(at, stub),
        naming(PAST),
        code(
            WINDOW,
            &[li(11, 75), NOP, PPC_SC_1, PPC_BLR, li(11, other), ba],
        ),
    ];
    ordinals(0, &segments)
}

#[test]
fn a_real_mode_bctr_to_a_loaded_address_puts_the_named_word_in_real_mode() {
    let stub = [ld_r3(), mtctr_r3(), bctr()];
    assert_eq!(past_site(VECTOR, 76, &stub), [(WINDOW + 8, None)]);
    assert_eq!(past_site(VECTOR, 75, &stub), [(WINDOW + 8, Some(75))]);
    assert_eq!(past_site(OUTSIDE, 76, &stub), [(WINDOW + 8, Some(75))]);
}

/// The site of a window at [`REAL_WINDOW`] that loads 75, with a call
/// to `at` whose return address is the kernel address of the window's
/// `nop`, and with `stub` at `at`.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
fn return_site(at: u64, stub: &[u32]) -> Vec<(u64, Option<u16>)> {
    let call = KERNEL + REAL_WINDOW;
    let bl = word(PpuInstruction::B {
        offset: i32::try_from(at.wrapping_sub(call).cast_signed()).expect("a near branch"),
        aa: false,
        link: true,
    });
    let segments = [
        code(at, stub),
        code(call, &[bl, NOP, NOP]),
        code(REAL_WINDOW, &[li(11, 75), NOP, PPC_SC_1]),
    ];
    ordinals(0, &segments)
}

#[test]
fn a_real_mode_blr_enters_the_real_alias_of_a_return_address() {
    assert_eq!(return_site(VECTOR, &[PPC_BLR]), [(REAL_SC, None)]);
    assert_eq!(return_site(OUTSIDE, &[PPC_BLR]), [(REAL_SC, Some(75))]);
}
