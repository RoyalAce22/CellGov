//! The firmware of an EHCI controller's rule: a setup register takes only
//! the setup words of its controller's firmware.

use cellgov_mem::{RegisterStore, RegisterWord};
use cellgov_ps3_abi::hw::ehci::{
    EhciSetupStore, EHCI_CONTROLLERS, EHCI_SETUP_STORES, INSNREG01, INSNREG03,
};

use super::{apply_store, INSN01, INSN03, REGISTERS, RULES, WORDS};

/// [OFW-4.93 lv2_kernel a:0x800000000023F964] stwx r6,r5,r9: the store of the register write helper.
const HELPER_STORE: u64 = 0x8000_0000_0023_F964;

/// [OFW-4.93 lv2_kernel a:0x8000000000245998-0x80000000002459AC] The kernel gives the helper the word 0x01000020.
const BURST_COUNTS: u32 = 0x0100_0020;

/// [OFW-4.93 lv2_kernel a:0x80000000002459B0-0x80000000002459C0] The kernel gives the helper the word 1.
const BURST_ENABLE: u32 = 1;

/// A firmware that no shipped table names.
const OTHER: &str = "another firmware";

/// A word that only [`TABLE`] gives, for `insnreg01` at [`OTHER`].
const OTHER_WORD: u32 = 0x0200_0040;

/// A test table: the two 4.93 setup stores, and one store at [`OTHER`]
/// of a word that 4.93 does not store.
const TABLE: &[EhciSetupStore] = &[
    EhciSetupStore {
        firmware: "4.93",
        register: INSNREG01,
        store: HELPER_STORE,
        value: BURST_COUNTS,
    },
    EhciSetupStore {
        firmware: "4.93",
        register: INSNREG03,
        store: HELPER_STORE,
        value: BURST_ENABLE,
    },
    EhciSetupStore {
        firmware: OTHER,
        register: INSNREG01,
        store: HELPER_STORE,
        value: OTHER_WORD,
    },
];

/// Whether a controller at `firmware` with the rows of `setup` takes the
/// code's own store of `value` to the unknown word at `index`.
fn takes(firmware: &str, setup: &[EhciSetupStore], index: usize, value: u32) -> bool {
    let mut words = [RegisterWord::UNKNOWN; WORDS];
    let store = RegisterStore {
        value,
        own: u32::MAX,
    };
    apply_store(firmware, setup, &[], &mut words, index, store)
}

#[test]
fn a_controller_of_another_firmware_refuses_the_4_93_setup_words() {
    for (index, value) in [(INSN01, BURST_COUNTS), (INSN03, BURST_ENABLE)] {
        assert!(takes("4.93", EHCI_SETUP_STORES, index, value), "{index}");
        assert!(!takes(OTHER, EHCI_SETUP_STORES, index, value), "{index}");
    }
}

#[test]
fn a_controller_of_another_firmware_takes_only_its_own_setup_word() {
    assert!(takes(OTHER, TABLE, INSN01, OTHER_WORD));
    assert!(!takes(OTHER, TABLE, INSN01, BURST_COUNTS));
    assert!(!takes(OTHER, TABLE, INSN03, BURST_ENABLE));
    assert!(!takes("4.93", TABLE, INSN01, OTHER_WORD));
    assert!(takes("4.93", TABLE, INSN01, BURST_COUNTS));
    assert!(takes("4.93", TABLE, INSN03, BURST_ENABLE));
}

#[test]
fn each_controller_rule_takes_the_setup_words_of_its_own_firmware() {
    assert_eq!(RULES.len(), EHCI_CONTROLLERS.len());
    assert!(EHCI_CONTROLLERS.iter().any(|c| c.firmware == "4.93"));
    let at_4_93: Vec<_> = EHCI_SETUP_STORES
        .iter()
        .filter(|s| s.firmware == "4.93")
        .collect();
    assert_eq!(at_4_93.len(), 2);
    assert!(at_4_93.iter().all(|s| s.store == HELPER_STORE));
    for (controller, rule) in EHCI_CONTROLLERS.iter().zip(RULES) {
        for row in EHCI_SETUP_STORES {
            let index = REGISTERS
                .iter()
                .position(|&r| r == row.register)
                .expect("a setup register of the file");
            let mut words = [RegisterWord::UNKNOWN; WORDS];
            let store = RegisterStore {
                value: row.value,
                own: u32::MAX,
            };
            assert_eq!(
                rule(&mut words, index, store),
                row.firmware == controller.firmware,
                "{controller:?} {row:?}"
            );
        }
    }
}
