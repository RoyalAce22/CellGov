use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_real_mode_switch_tests::{
    lis_sf, mtmsrd, mtspr, ori, site, window, OUTSIDE, RFID, SC, WINDOW,
};
use super::hypercall_tests::li;
use super::*;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_IR, SPR_SRR0, SPR_SRR1};

/// The kernel address of the segment that [`mtmsrd_stub`] gives.
const SECOND: u64 = OUTSIDE + 0x1000;

/// The address of the `mtmsrd` in [`mtmsrd_stub`].
const MTMSRD: u64 = SECOND + 4;

/// `lis r3,-32768; rldicr r3,r3,32,31; ori r3,r3,low`: `address` in r3,
/// for a kernel address whose bits 0:47 are those of the kernel base.
fn build_kernel(address: u64) -> [u32; 3] {
    [
        word(PpuInstruction::Addis {
            rt: 3,
            ra: 0,
            imm: i16::MIN,
        }),
        word(PpuInstruction::Rldicr {
            ra: 3,
            rs: 3,
            sh: 32,
            me: 31,
            rc: false,
        }),
        ori(3, address & 0xFFFF),
    ]
}

/// A stub at [`OUTSIDE`] that runs `before`, then gives SRR0 `srr0` and
/// SRR1 the value SF | IR, then runs `rfid`.
fn rfid_ir_set(before: &[u32], srr0: u64) -> Segment {
    let mut words = before.to_vec();
    words.extend(build_kernel(srr0));
    words.extend([lis_sf(4), ori(4, MSR_IR)]);
    words.extend([mtspr(SPR_SRR0, 3), mtspr(SPR_SRR1, 4), RFID]);
    code(OUTSIDE, &words)
}

/// A stub at [`SECOND`]: `li r5,MSR_IR; mtmsrd r5,0`, then a branch to
/// the path past the window's `blr`.
fn mtmsrd_stub() -> Segment {
    let to_past = WINDOW + 0x10;
    let at = SECOND + 8;
    let b = word(PpuInstruction::B {
        offset: i32::try_from(to_past.wrapping_sub(at).cast_signed()).expect("a near branch"),
        aa: false,
        link: false,
    });
    let msr_ir = li(5, i16::try_from(MSR_IR).expect("a low MSR bit"));
    code(SECOND, &[msr_ir, mtmsrd(5, false), b])
}

/// SRR0 has bit 62 or 63 set and names the window's `nop`, which lies
/// between its `li r11` and its `sc`.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
#[test]
fn an_rfid_outside_real_mode_to_srr0_with_low_bits_set_enters_its_word() {
    for low_bits in 1..=3 {
        assert_eq!(
            site(76, rfid_ir_set(&[], WINDOW + 4 + low_bits)),
            [(SC, None)],
            "{low_bits}"
        );
    }
    assert_eq!(site(76, rfid_ir_set(&[], WINDOW + 1)), [(SC, Some(75))]);
}

/// Stub A gives r5 = 0, then its `rfid` enters the `mtmsrd r5,0` at
/// [`MTMSRD`] with IR at 1. The fall-through reaches the `mtmsrd` with
/// r5 = MSR_IR, but from stub A the `mtmsrd` clears IR. Then the path
/// past the window's `blr` runs in real mode, and its `ba` reaches the
/// window's `nop` with r11 = 76.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\] and MSR\[SF\] to RS bit 0 | RS bit 1.
#[test]
fn an_rfid_outside_real_mode_to_srr0_with_low_bits_set_is_a_join_of_the_switch_scan() {
    let census = |srr0: u64| {
        let stub_a = rfid_ir_set(&[li(5, 0)], srr0);
        ordinals(0, &[stub_a, mtmsrd_stub(), window(76)])
    };
    assert_eq!(census(MTMSRD | 1), [(SC, None)]);
    assert_eq!(census((MTMSRD + 4) | 1), [(SC, Some(75))]);
}
