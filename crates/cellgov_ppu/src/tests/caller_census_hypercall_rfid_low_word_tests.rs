use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_real_mode_switch_tests::{
    lis_sf, mtmsrd, mtspr, ori, window, OUTSIDE, RFID, SC,
};
use super::hypercall_tests::{li, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_IR, SPR_SRR0, SPR_SRR1};

/// The address of a window below 4 GiB: `li r11,75; nop; sc; blr`.
const LOW_WINDOW: u64 = 0x3000;

/// The address of the low window's `sc`.
const LOW_SC: u64 = LOW_WINDOW + 8;

/// The address of the segment that [`low_mtmsrd_stub`] gives.
const LOW_STUB: u64 = 0x6000;

/// `li r3,1; rldicr r3,r3,32,31; ori r3,r3,low`: 0x1_0000_0000 | `low`
/// in r3.
///
/// Real mode ignores only EA bits 0 and 1, so no real alias of the value
/// is a word below 4 GiB.
fn build_high(low: u16) -> [u32; 3] {
    [
        li(3, 1),
        word(PpuInstruction::Rldicr {
            ra: 3,
            rs: 3,
            sh: 32,
            me: 31,
            rc: false,
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: low,
        }),
    ]
}

/// A stub at [`OUTSIDE`] that runs `before`, builds 0x1_0000_0000 | `low`
/// in SRR0 and the value that `srr1` builds in r4 in SRR1, then runs
/// `rfid`.
fn rfid_high(before: &[u32], low: u16, srr1: &[u32]) -> Segment {
    let mut words = before.to_vec();
    words.extend(build_high(low));
    words.extend_from_slice(srr1);
    words.extend([mtspr(SPR_SRR0, 3), mtspr(SPR_SRR1, 4), RFID]);
    code(OUTSIDE, &words)
}

/// `li r4,MSR_IR`: SRR1 with IR at 1 and SF at 0.
fn srr1_ir() -> [u32; 1] {
    [li(4, i16::try_from(MSR_IR).expect("a low MSR bit"))]
}

/// A stub at [`LOW_STUB`]: `li r5,MSR_IR; mtmsrd r5,0`, then `ba` to the
/// real address of the path past the window's `blr`.
fn low_mtmsrd_stub() -> Segment {
    let ba = word(PpuInstruction::B {
        offset: 0x3010,
        aa: true,
        link: false,
    });
    let msr_ir = li(5, i16::try_from(MSR_IR).expect("a low MSR bit"));
    code(LOW_STUB, &[msr_ir, mtmsrd(5, false), ba])
}

/// SRR0 names the low window's `nop` in its low word, and SRR1 keeps IR
/// at 1. With SF at 0 the `rfid` enters the `nop`, which lies between
/// the `li r11` and the `sc`.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
#[test]
fn an_rfid_outside_real_mode_with_sf_at_0_enters_the_low_word_of_srr0() {
    let census = |low: u16, srr1: &[u32]| {
        let window = code(LOW_WINDOW, &[li(11, 75), NOP, PPC_SC_1, PPC_BLR]);
        ordinals(0, &[rfid_high(&[], low, srr1), window])
    };
    let sf_ir = [lis_sf(4), ori(4, MSR_IR)];
    assert_eq!(census(0x3004, &srr1_ir()), [(LOW_SC, None)]);
    assert_eq!(census(0x3000, &srr1_ir()), [(LOW_SC, Some(75))]);
    assert_eq!(census(0x3004, &sf_ir), [(LOW_SC, Some(75))]);
}

/// The `mtmsrd r5,0` of [`low_mtmsrd_stub`] is a join of two paths:
///
/// - the fall-through, with r5 = `MSR_IR`;
/// - the `rfid` of stub A, with SF at 0, IR at 1 and r5 = 0.
///
/// On the second path the `mtmsrd` clears IR. The `ba` then runs in real
/// mode and reaches the path past the window's `blr`, which enters the
/// window's `nop` with r11 = 76.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\] and MSR\[SF\] to RS bit 0 | RS bit 1.
#[test]
fn an_rfid_outside_real_mode_with_sf_at_0_is_a_join_of_the_switch_scan() {
    let census = |low: u16| {
        let stub_a = rfid_high(&[li(5, 0)], low, &srr1_ir());
        ordinals(0, &[stub_a, low_mtmsrd_stub(), window(76)])
    };
    assert_eq!(census(0x6004), [(SC, None)]);
    assert_eq!(census(0x6008), [(SC, Some(75))]);
}
