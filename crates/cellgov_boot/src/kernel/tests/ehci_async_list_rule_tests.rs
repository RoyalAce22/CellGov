//! The rule of ASYNCLISTADDR: a store lands only while both asynchronous
//! schedule bits are known to be 0.

use cellgov_mem::{RegisterStore, RegisterWord};
use cellgov_ps3_abi::hw::ehci::{USBCMD_ASE, USBSTS_ASS};

use super::{apply_store, ASYNC, CMD, STS, WORDS};

/// A word with bits 4:0 set, which the register reads as 0.
const STORED: u32 = 0x1234_567F;

/// [EHCI p:25 s:2.3.7] Bits 4:0 always read as zero.
const HELD: u32 = 0x1234_5660;

/// The words after a store of [`STORED`] to ASYNCLISTADDR with USBCMD at
/// `cmd` and USBSTS at `sts`, or `None` when the rule refuses it.
fn after(cmd: RegisterWord, sts: RegisterWord) -> Option<[RegisterWord; WORDS]> {
    let mut words = [RegisterWord::UNKNOWN; WORDS];
    words[CMD] = cmd;
    words[STS] = sts;
    let store = RegisterStore {
        value: STORED,
        own: u32::MAX,
    };
    apply_store("4.93", &[], &[], &mut words, ASYNC, store).then_some(words)
}

#[test]
fn a_store_while_the_schedule_is_known_disabled_holds_the_word_with_bits_4_to_0_clear() {
    let words = after(RegisterWord::known(0), RegisterWord::known(0)).expect("taken");
    assert_eq!(words[ASYNC], RegisterWord::known(HELD));
    assert_eq!(words[CMD], RegisterWord::known(0));
    assert_eq!(words[STS], RegisterWord::known(0));
}

#[test]
fn a_store_while_a_schedule_bit_is_set_or_unknown_is_refused() {
    let unknown_bit = |bit: u32| RegisterWord {
        value: 0,
        known: !bit,
    };
    // (what the state is, USBCMD, USBSTS)
    let cases: &[(&str, RegisterWord, RegisterWord)] = &[
        (
            "enable set",
            RegisterWord::known(USBCMD_ASE),
            RegisterWord::known(0),
        ),
        (
            "enable unknown",
            unknown_bit(USBCMD_ASE),
            RegisterWord::known(0),
        ),
        (
            "status set",
            RegisterWord::known(0),
            RegisterWord::known(USBSTS_ASS),
        ),
        (
            "status unknown",
            RegisterWord::known(0),
            unknown_bit(USBSTS_ASS),
        ),
    ];
    assert_eq!(cases.len(), 4);
    for &(what, cmd, sts) in cases {
        assert_eq!(after(cmd, sts), None, "{what}");
    }
}

#[test]
fn a_store_with_a_bit_that_is_not_the_code_s_own_is_refused() {
    let mut words = [RegisterWord::known(0); WORDS];
    let store = RegisterStore {
        value: STORED,
        own: !1,
    };
    assert!(!apply_store("4.93", &[], &[], &mut words, ASYNC, store));
    assert_eq!(words[ASYNC], RegisterWord::known(0));
}
