use super::hypercall_entry_tests::{code, data_bytes, ordinals, word, Segment};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1;

/// The address of a window `li r11,75; nop; sc`, which `lwa` gives for
/// the 32-bit word 0x8000_3000.
const WINDOW: u64 = 0xFFFF_FFFF_8000_3000;

/// The window's `sc`.
const SC: u64 = WINDOW + 8;

fn window() -> Segment {
    code(WINDOW, &[li(11, 75), NOP, PPC_SC_1])
}

/// `lwa r3,0(r4); mtctr r3; bctr` at a word outside the interrupt
/// vectors, which runs in virtual mode.
fn loaded_bctr() -> Segment {
    code(
        KERNEL + 0x5000,
        &[
            word(PpuInstruction::Lwa {
                rt: 3,
                ra: 4,
                imm: 0,
            }),
            word(PpuInstruction::Mtctr { rs: 3 }),
            word(PpuInstruction::Bcctr {
                bo: 0b10100,
                bi: 0,
                link: false,
            }),
        ],
    )
}

/// The 32-bit word 0x8000_3004 between two zero words. No doubleword of
/// it and no zero extension of it names a word of the code.
fn word_between_zeros() -> Segment {
    let mut bytes = vec![0; 4];
    bytes.extend(0x8000_3004_u32.to_be_bytes());
    bytes.extend([0; 4]);
    data_bytes(KERNEL + 0x2_0000, bytes)
}

/// `lwa` can load 0x8000_3004 as 0xFFFF_FFFF_8000_3004, and the `bctr`
/// then enters the window's `nop` with r11 unknown.
///
/// [PPC-Book1 p:38 s:3.3.2] `lwa` fills RT bits 0:31 with a copy of bit 0 of the loaded word.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00, with the high 32 bits at 0 only in 32-bit mode.
#[test]
fn a_bctr_enters_the_word_that_the_sign_extension_of_a_32_bit_word_names() {
    assert_eq!(ordinals(0, &[loaded_bctr(), window()]), [(SC, Some(75))]);
    let segments = [loaded_bctr(), window(), word_between_zeros()];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
}

/// The address of a window `li r11,75; nop; sc`, which `lwz` gives for
/// the 32-bit word 0x8000_3000.
const LOW_WINDOW: u64 = 0x8000_3000;

/// The window's `sc`.
const LOW_SC: u64 = LOW_WINDOW + 8;

fn low_window() -> Segment {
    code(LOW_WINDOW, &[li(11, 75), NOP, PPC_SC_1])
}

/// `lwz r3,0(r4); mtctr r3; bctr` at the word of [`loaded_bctr`].
fn loaded_zero_extended_bctr() -> Segment {
    code(
        KERNEL + 0x5000,
        &[
            word(PpuInstruction::Lwz {
                rt: 3,
                ra: 4,
                imm: 0,
            }),
            word(PpuInstruction::Mtctr { rs: 3 }),
            word(PpuInstruction::Bcctr {
                bo: 0b10100,
                bi: 0,
                link: false,
            }),
        ],
    )
}

/// The 32-bit word 0x8000_3004 between two words 0x0101_0101. Each
/// doubleword of it has a nonzero high half, so in 64-bit mode only the
/// zero extension of the word names the window's `nop`.
fn word_between_fillers() -> Segment {
    let mut bytes = 0x0101_0101_u32.to_be_bytes().to_vec();
    bytes.extend(0x8000_3004_u32.to_be_bytes());
    bytes.extend(0x0101_0101_u32.to_be_bytes());
    data_bytes(KERNEL + 0x2_0000, bytes)
}

/// `lwz` fills RT bits 0:31 with zeros. So it can load 0x8000_3004 as
/// 0x0000_0000_8000_3004, and the `bctr` then enters the window's `nop`
/// with r11 unknown.
///
/// [PPC-Book1 p:37 s:3.3.2] `lwz` loads the word into RT 32:63 and sets RT 0:31 to 0.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_enters_the_word_that_the_zero_extension_of_a_32_bit_word_with_bit_0_set_names() {
    let without = [loaded_zero_extended_bctr(), low_window()];
    assert_eq!(ordinals(0, &without), [(LOW_SC, Some(75))]);
    let segments = [
        loaded_zero_extended_bctr(),
        low_window(),
        word_between_fillers(),
    ];
    assert_eq!(ordinals(0, &segments), [(LOW_SC, None)]);
}

/// A data segment that starts one byte past a word boundary, with the
/// 32-bit word 0x8000_3004 between two zero words from its first
/// word-aligned address on. A word that starts at any other offset names
/// no word of the code.
fn word_after_an_unaligned_start() -> Segment {
    let mut bytes = vec![0; 3];
    bytes.extend([0; 4]);
    bytes.extend(0x8000_3004_u32.to_be_bytes());
    bytes.extend([0; 4]);
    data_bytes(KERNEL + 0x2_0001, bytes)
}

#[test]
fn a_bctr_enters_the_word_that_a_32_bit_word_after_an_unaligned_segment_start_names() {
    assert_eq!(ordinals(0, &[loaded_bctr(), window()]), [(SC, Some(75))]);
    let segments = [loaded_bctr(), window(), word_after_an_unaligned_start()];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
}
