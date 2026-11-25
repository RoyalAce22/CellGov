use super::hypercall_ctr_decrement_tests::VECTOR;
use super::hypercall_entry_tests::{code, data_bytes, ordinals, pointers, word, Segment};
use super::hypercall_jump_table_alias_tests::{branch_through_table, build_low, window, CODE, END};
use super::hypercall_tests::{li, KERNEL};
use super::*;

/// A data segment at `at` that holds the 32-bit words `words`.
fn words(at: u64, words: &[u32]) -> Segment {
    data_bytes(at, words.iter().flat_map(|w| w.to_be_bytes()).collect())
}

/// The code builds 0xFFFF_FFFF_FFFF_FFF8, so 32-bit code reads the table
/// at 0xFFFF_FFF8. Its first two entries reach the code's first word.
/// Its third entry is at 0x0 and reaches the window's `nop` at 0x5004.
/// The values 0x1_0008 and 0x500C name no word of the window. A code
/// pointer names the code.
///
/// [PPC-Book1 p:15 s:1.12.2] Effective address arithmetic wraps from 2^64 - 1 to 0 in 64-bit mode, and from 2^32 - 1 to 0 in 32-bit mode.
#[test]
fn a_32_bit_jump_table_continues_at_0_after_the_last_word_below_4_gib() {
    let segments = [
        branch_through_table(CODE, &[li(3, -8)]),
        words(0xFFFF_FFF8, &[0x0001_0008, 0x0001_0008]),
        words(0x0, &[0x500C, END]),
        window(0x5000),
        pointers(&[CODE]),
    ];
    assert_eq!(ordinals(0, &segments), [(0x5008, None)]);
}

/// Real-mode code builds the base 0x4000. The file bytes hold the first
/// entry at KERNEL + 0x4000 and the second entry at 0x4004, which has
/// the real address of KERNEL + 0x4004. The second entry reaches the
/// window's `nop`.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
/// [PPC-Book3 p:30 s:4.2.6.2] The offset real mode mechanism compares EA bits 2:63 with the RMLR limit and ORs them with the RMOR offset.
#[test]
fn a_jump_table_entry_is_read_at_each_address_with_its_real_address() {
    let at = KERNEL + 0x5000;
    let segments = [
        branch_through_table(VECTOR, &build_low(0x4000)),
        words(KERNEL + 0x4000, &[0x1000]),
        words(0x4004, &[0x1004, END]),
        window(at),
    ];
    assert_eq!(ordinals(0, &segments), [(at + 8, None)]);
}

/// A table after the `bctr` at KERNEL + 0x1000 has the entry -0x2000.
/// The start plus the entry is 0x7FFF_FFFF_FFFF_F004, which no segment
/// holds at any real alias. Its low-order 32 bits have the real address
/// of the window's `nop` at KERNEL + 0xFFFF_F004.
///
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_jump_table_after_a_bcctr_enters_the_low_word_of_each_target() {
    let at = KERNEL + 0xFFFF_F000;
    let bctr = word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    });
    let segments = [
        code(KERNEL + 0x1000, &[bctr]),
        words(KERNEL + 0x1004, &[(-0x2000i32).cast_unsigned(), END]),
        window(at),
    ];
    assert_eq!(ordinals(0, &segments), [(at + 8, None)]);
}
