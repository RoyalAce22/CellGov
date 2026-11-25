//! The rule of PERIODICLISTBASE: a store lands only while both periodic
//! schedule bits are known to be 0.

use cellgov_mem::{RegisterStore, RegisterWord};
use cellgov_ps3_abi::hw::ehci::{USBCMD_PSE, USBSTS_PSS};

use super::{apply_store, CMD, PERIODIC, STS, WORDS};

/// A 4-Kbyte aligned frame list address.
const STORED: u32 = 0x1234_5000;

/// [EHCI p:25 s:2.3.6] Bits 31:12 hold the base address; bits 11:0 are undefined during runtime.
const KNOWN: u32 = 0xFFFF_F000;

/// The words after a store of `value` to PERIODICLISTBASE with USBCMD at
/// `cmd` and USBSTS at `sts`, or `None` when the rule refuses it.
fn after(cmd: RegisterWord, sts: RegisterWord, value: u32) -> Option<[RegisterWord; WORDS]> {
    let mut words = [RegisterWord::UNKNOWN; WORDS];
    words[CMD] = cmd;
    words[STS] = sts;
    let store = RegisterStore {
        value,
        own: u32::MAX,
    };
    apply_store("4.93", &[], &[], &mut words, PERIODIC, store).then_some(words)
}

#[test]
fn a_store_while_the_periodic_schedule_is_known_disabled_holds_bits_31_to_12() {
    let words = after(RegisterWord::known(0), RegisterWord::known(0), STORED).expect("taken");
    assert_eq!(
        words[PERIODIC],
        RegisterWord {
            value: STORED,
            known: KNOWN,
        }
    );
    assert_eq!(words[CMD], RegisterWord::known(0));
    assert_eq!(words[STS], RegisterWord::known(0));
}

#[test]
fn a_store_to_periodiclistbase_changes_no_other_word() {
    // Each other word holds a distinct known value; USBCMD and USBSTS
    // hold 0 so that the rule takes the store.
    let mut words = [RegisterWord::UNKNOWN; WORDS];
    for (index, word) in words.iter_mut().enumerate() {
        if index != CMD && index != STS {
            *word = RegisterWord::known(0x100 + index as u32);
        }
    }
    words[CMD] = RegisterWord::known(0);
    words[STS] = RegisterWord::known(0);
    let mut expected = words;
    expected[PERIODIC] = RegisterWord {
        value: STORED,
        known: KNOWN,
    };
    let store = RegisterStore {
        value: STORED,
        own: u32::MAX,
    };
    assert!(apply_store("4.93", &[], &[], &mut words, PERIODIC, store));
    assert_eq!(words, expected);
}

#[test]
fn a_store_while_a_periodic_schedule_bit_is_set_or_unknown_is_refused() {
    let unknown_bit = |bit: u32| RegisterWord {
        value: 0,
        known: !bit,
    };
    // (what the state is, USBCMD, USBSTS)
    let cases: &[(&str, RegisterWord, RegisterWord)] = &[
        (
            "enable set",
            RegisterWord::known(USBCMD_PSE),
            RegisterWord::known(0),
        ),
        (
            "enable unknown",
            unknown_bit(USBCMD_PSE),
            RegisterWord::known(0),
        ),
        (
            "status set",
            RegisterWord::known(0),
            RegisterWord::known(USBSTS_PSS),
        ),
        (
            "status unknown",
            RegisterWord::known(0),
            unknown_bit(USBSTS_PSS),
        ),
    ];
    assert_eq!(cases.len(), 4);
    for &(what, cmd, sts) in cases {
        assert_eq!(after(cmd, sts, STORED), None, "{what}");
    }
}

#[test]
fn a_store_with_a_reserved_bit_set_is_refused() {
    // [EHCI p:25 s:2.3.6] Bits 11:0 must be written as 0s.
    let reserved: &[u32] = &[1 << 0, 1 << 11];
    assert_eq!(reserved.len(), 2);
    for &bit in reserved {
        let known = RegisterWord::known(0);
        assert_eq!(after(known, known, STORED | bit), None, "{bit:#x}");
    }
}

#[test]
fn a_periodic_store_with_a_bit_that_is_not_the_code_s_own_is_refused() {
    let mut words = [RegisterWord::known(0); WORDS];
    let store = RegisterStore {
        value: STORED,
        own: !(1 << 12),
    };
    assert!(!apply_store("4.93", &[], &[], &mut words, PERIODIC, store));
    assert_eq!(words[PERIODIC], RegisterWord::known(0));
}
