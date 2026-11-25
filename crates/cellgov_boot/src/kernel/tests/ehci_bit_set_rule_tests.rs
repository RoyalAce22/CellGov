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
