//! The firmware of a setup register bit set: `insnreg02` takes the set
//! of bit 0x800 only at the firmware of its row.

use cellgov_mem::{RegisterStore, RegisterWord};
use cellgov_ps3_abi::hw::ehci::EHCI_SETUP_BIT_SETS;

use super::{apply_store, INSN02, WORDS};

/// [OFW-4.93 lv2_kernel a:0x8000000000245F18] ori r0,r0,0x800: the bit that the kernel sets.
const BIT: u32 = 0x800;

/// Whether a controller at `firmware` takes the store of the unknown
/// `insnreg02` word with [`BIT`] set and every other bit passed back.
fn takes(firmware: &str) -> bool {
    let mut words = [RegisterWord::UNKNOWN; WORDS];
    let store = RegisterStore {
        value: BIT,
        own: BIT,
    };
    apply_store(
        firmware,
        &[],
        EHCI_SETUP_BIT_SETS,
        &mut words,
        INSN02,
        store,
    )
}

#[test]
fn only_a_4_93_controller_takes_the_insnreg02_bit_set() {
    assert!(takes("4.93"));
    assert!(!takes("another firmware"));
}

/// Applies a store of `value` that the code sets entirely itself to an
/// `insnreg02` word of which every bit is known and 0 except bit 0, which
/// is 1. Returns whether the rule takes it, and the word afterwards.
fn store_to_known_word(value: u32) -> (bool, RegisterWord) {
    let mut words = [RegisterWord::UNKNOWN; WORDS];
    words[INSN02] = RegisterWord::known(1);
    let store = RegisterStore {
        value,
        own: u32::MAX,
    };
    let taken = apply_store("4.93", &[], EHCI_SETUP_BIT_SETS, &mut words, INSN02, store);
    (taken, words[INSN02])
}

#[test]
fn a_store_that_changes_a_known_bit_outside_the_row_is_refused() {
    assert_eq!(
        store_to_known_word(1 | BIT),
        (true, RegisterWord::known(1 | BIT))
    );
    assert_eq!(store_to_known_word(BIT), (false, RegisterWord::known(1)));
}

#[test]
fn a_store_that_sets_an_unknown_bit_of_its_own_is_refused() {
    let mut words = [RegisterWord::UNKNOWN; WORDS];
    let store = RegisterStore {
        value: BIT,
        own: u32::MAX,
    };
    let taken = apply_store("4.93", &[], EHCI_SETUP_BIT_SETS, &mut words, INSN02, store);
    assert!(!taken);
    assert_eq!(words[INSN02], RegisterWord::UNKNOWN);
}
