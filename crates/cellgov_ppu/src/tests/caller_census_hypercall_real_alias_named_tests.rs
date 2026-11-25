use super::hypercall_ctr_decrement_tests::{window as plain_window, VECTOR};
use super::hypercall_entry_tests::{code, data_bytes, ordinals, word, Segment};
use super::hypercall_real_mode_switch_tests::{mtspr, window, OUTSIDE, PAST, RFID, SC};
use super::hypercall_tests::KERNEL;
use super::*;
use cellgov_ps3_abi::hw::ppu_supervisor::{SPR_SRR0, SPR_SRR1};

/// A word that runs in virtual mode unless a switch that the scan does
/// not know puts control into real mode.
const VIRTUAL: u64 = KERNEL + 0x6000;

/// The real address of the plain window's `nop`.
const NOP_REAL: u64 = 0x3004;

/// A data segment that holds the doubleword `address`, a real address
/// that no segment holds at that address.
fn doubleword(address: u64) -> Segment {
    data_bytes(KERNEL + 0x2_0000, address.to_be_bytes().to_vec())
}

/// `ld rt,0(ra)`: a value that the scan does not know.
fn ld(rt: u8, ra: u8) -> u32 {
    word(PpuInstruction::Ld { rt, ra, imm: 0 })
}

/// `ld r3,0(r4); mtctr r3; bctr` at `at`: a branch to an address that
/// the code loads from memory.
fn loaded_bctr(at: u64) -> Segment {
    code(
        at,
        &[
            ld(3, 4),
            word(PpuInstruction::Mtctr { rs: 3 }),
            word(PpuInstruction::Bcctr {
                bo: 0b10100,
                bi: 0,
                link: false,
            }),
        ],
    )
}

/// `ld r3,0(r1); ld r4,0(r1)`, then r3 into SRR0 and r4 into SRR1, then
/// `rfid`: a switch to an address and a mode that the scan does not know.
fn unbounded_rfid() -> Segment {
    code(
        OUTSIDE,
        &[
            ld(3, 1),
            ld(4, 1),
            mtspr(SPR_SRR0, 3),
            mtspr(SPR_SRR1, 4),
            RFID,
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
fn a_real_mode_bctr_enters_the_word_at_the_real_address_of_a_doubleword_no_segment_holds() {
    let segments = [loaded_bctr(VECTOR), window(76), doubleword(u64::from(PAST))];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
}

/// The unbounded `rfid` can put each word into real mode, so the `bctr`
/// can load 0x3004 and enter the window's `nop` with r11 unset.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn under_an_unbounded_rfid_a_bctr_enters_the_word_at_the_real_address_of_a_doubleword() {
    let without = [loaded_bctr(VIRTUAL), unbounded_rfid(), plain_window(&[])];
    assert_eq!(ordinals(0, &without), [(SC, Some(75))]);
    let segments = [
        loaded_bctr(VIRTUAL),
        unbounded_rfid(),
        plain_window(&[]),
        doubleword(NOP_REAL),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None)]);
}

/// No `bcctr` or `bclr` runs in real mode, and a branch in virtual mode
/// reaches only the word at the address itself.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_virtual_mode_bctr_leaves_the_word_at_the_real_address_of_a_doubleword_unentered() {
    let segments = [
        loaded_bctr(VIRTUAL),
        plain_window(&[]),
        doubleword(NOP_REAL),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, Some(75))]);
}
