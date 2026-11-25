use super::hypercall_entry_tests::{code, ordinals, word};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1;

/// The address of the `bctr` that the tests put before the table.
const CODE: u64 = KERNEL + 0x1_0000;

/// The address of the table: the word after the `bctr`.
const START: u64 = CODE + 4;

/// A table entry that reaches no word of the code, which ends the run.
const END: u32 = 0x0010_0000;

/// `bctr`, then a relative jump table with one entry for each of
/// `targets` and an [`END`] entry, then `li r11,74; nop; sc 1`.
fn table_then_window(targets: &[u64]) -> Vec<(u64, Option<u16>)> {
    let mut words = vec![word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })];
    words.extend(
        targets
            .iter()
            .map(|&target| u32::try_from(target - START).expect("a small offset")),
    );
    words.extend([END, li(11, 74), NOP, PPC_SC_1]);
    ordinals(0, &[code(CODE, &words)])
}

/// The address of the window's `li r11` after a table of `entries`
/// entries.
fn load(entries: u64) -> u64 {
    START + 4 * (entries + 1)
}

/// The target has bit 62 or 63 set. With them cleared, it names the
/// window's `nop`, which lies between its `li r11` and its `sc`.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_jump_table_target_with_low_bits_set_enters_its_word() {
    let sc = load(1) + 8;
    for low_bits in 1..=3 {
        assert_eq!(
            table_then_window(&[load(1) + 4 + low_bits]),
            [(sc, None)],
            "{low_bits}"
        );
    }
    assert_eq!(table_then_window(&[load(1) + 1]), [(sc, Some(74))]);
}

/// The first entry has bit 62 or 63 set, and with them cleared it names
/// the window's `li r11`. The second entry names the window's `nop`.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_jump_table_target_with_low_bits_set_does_not_end_the_run() {
    let sc = load(2) + 8;
    for low_bits in 1..=3 {
        assert_eq!(
            table_then_window(&[load(2) + low_bits, load(2) + 4]),
            [(sc, None)],
            "{low_bits}"
        );
    }
    assert_eq!(table_then_window(&[load(2) + 1, load(2)]), [(sc, Some(74))]);
}
