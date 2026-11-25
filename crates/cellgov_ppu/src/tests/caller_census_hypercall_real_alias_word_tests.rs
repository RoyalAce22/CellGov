use super::hypercall_ctr_decrement_tests::VECTOR;
use super::hypercall_entry_tests::{code, data_bytes, ordinals, word, Segment};
use super::hypercall_real_mode_switch_tests::{window, PAST, SC};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1;

/// A data segment that holds the 32-bit word `value` between two words of
/// all ones. No segment holds an address that a doubleword of it gives.
fn word_between(value: u32) -> Segment {
    let mut bytes = u32::MAX.to_be_bytes().to_vec();
    bytes.extend(value.to_be_bytes());
    bytes.extend(u32::MAX.to_be_bytes());
    data_bytes(KERNEL + 0x2_0000, bytes)
}

/// `lwz r3,0(r4); mtctr r3; bctr` at `at`: a branch to a 32-bit value
/// that the code loads from memory.
fn loaded_bctr(at: u64) -> Segment {
    code(
        at,
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

/// The vector's `bctr` can load 0x3010 and reach the word past the
/// window's `blr` in real mode. That path enters the window's `nop` with
/// r11 = 76.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_real_mode_bctr_enters_the_word_at_the_real_address_of_a_32_bit_word() {
    let without = [loaded_bctr(VECTOR), window(76)];
    assert_eq!(ordinals(0, &without), [(SC, Some(75))]);
    let segments = [
        loaded_bctr(VECTOR),
        window(76),
        word_between(u32::from(PAST)),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
}

/// The vector's `bctr` ignores the two low-order bits of 0x3012, so it
/// reaches the word past the window's `blr` in real mode.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_real_mode_bctr_enters_the_word_that_a_32_bit_word_with_low_bits_set_names() {
    let segments = [
        loaded_bctr(VECTOR),
        window(76),
        word_between(u32::from(PAST) + 2),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
}

/// The low half of the doubleword KERNEL + 0x3004 is 0x3004. A `lwz`
/// can load it, and a `bctr` in virtual mode then reaches the word at
/// 0x3004, between the `li r11` and the `sc` of the window at 0x3000.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_virtual_mode_bctr_enters_the_word_that_the_low_half_of_a_doubleword_names() {
    let segments = [
        loaded_bctr(KERNEL + 0x5000),
        code(0x3000, &[li(11, 75), NOP, PPC_SC_1]),
        data_bytes(KERNEL + 0x2_0000, (KERNEL + 0x3004).to_be_bytes().to_vec()),
    ];
    assert_eq!(ordinals(0, &segments), [(0x3008, None)]);
}
