use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_real_mode_switch_tests::{
    lis_sf, mtmsr, mtmsrd, mtspr, ori, window, RFID, SC,
};
use super::hypercall_tests::{li, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_IR, SPR_SRR0, SPR_SRR1};

/// The address of a window below 4 GiB: `li r11,75; nop; sc; blr`.
const LOW_WINDOW: u64 = 0x3000;

/// The address of the low window's `sc`.
const LOW_SC: u64 = LOW_WINDOW + 8;

/// The address of the stub that runs the switch to 32-bit mode.
const STUB: u64 = 0x6000;

/// The address of a second stub.
const OTHER_STUB: u64 = 0x7000;

/// The low window.
fn low_window() -> Segment {
    code(LOW_WINDOW, &[li(11, 75), NOP, PPC_SC_1, PPC_BLR])
}

/// `li rt,1; rldicr rt,rt,32,31; ori rt,rt,low`: 0x1_0000_0000 | `low`
/// in `rt`.
///
/// Real mode ignores only EA bits 0 and 1, so no real alias of the value
/// is a word below 4 GiB.
fn build_high(rt: u8, low: u16) -> [u32; 3] {
    [
        li(rt, 1),
        word(PpuInstruction::Rldicr {
            ra: rt,
            rs: rt,
            sh: 32,
            me: 31,
            rc: false,
        }),
        word(PpuInstruction::Ori {
            ra: rt,
            rs: rt,
            imm: low,
        }),
    ]
}

/// `li r5,MSR_IR`: an MSR value with IR at 1 and SF at 0.
fn msr_ir() -> Vec<u32> {
    vec![li(5, i16::try_from(MSR_IR).expect("a low MSR bit"))]
}

/// `lis r5,-0x8000; ori r5,r5,MSR_IR`: an MSR value with SF and IR at 1.
fn msr_sf_ir() -> Vec<u32> {
    vec![lis_sf(5), ori(5, MSR_IR)]
}

/// `bctr`.
fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// `b .`: a branch to itself.
fn spin() -> u32 {
    word(PpuInstruction::B {
        offset: 0,
        aa: false,
        link: false,
    })
}

/// The words of `msr`, then `mtmsrd r5,0`, then the words of `rest`.
fn mtmsrd_stub(msr: &[u32], rest: &[u32]) -> Segment {
    let mut words = msr.to_vec();
    words.push(mtmsrd(5, false));
    words.extend_from_slice(rest);
    code(STUB, &words)
}

/// `rest` after the words that build 0x1_0000_0000 | `low` in r3.
fn after_build_high(low: u16, rest: &[u32]) -> Vec<u32> {
    let mut words = build_high(3, low).to_vec();
    words.extend_from_slice(rest);
    words
}

/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\] and MSR\[SF\] to RS bit 0 | RS bit 1.
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
#[test]
fn a_bcctr_or_bclr_after_an_mtmsrd_that_clears_sf_enters_the_low_word_of_its_target() {
    let census = |msr: &[u32], branch: [u32; 2]| {
        let rest = after_build_high(0x3004, &branch);
        ordinals(0, &[mtmsrd_stub(msr, &rest), low_window()])
    };
    let bcctr = [word(PpuInstruction::Mtctr { rs: 3 }), bctr()];
    let bclr = [word(PpuInstruction::Mtlr { rs: 3 }), PPC_BLR];
    assert_eq!(census(&msr_ir(), bcctr), [(LOW_SC, None)]);
    assert_eq!(census(&msr_ir(), bclr), [(LOW_SC, None)]);
    assert_eq!(census(&msr_sf_ir(), bcctr), [(LOW_SC, Some(75))]);
    assert_eq!(census(&msr_sf_ir(), bclr), [(LOW_SC, Some(75))]);
}

/// The stub builds 0x1_0000_3004 in r3, then runs `bctr` with a CTR that
/// it loads, which can hold that address.
///
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
#[test]
fn a_bcctr_of_a_loaded_ctr_in_32_bit_mode_enters_the_low_word_of_a_built_constant() {
    let census = |msr: &[u32]| {
        let ld = word(PpuInstruction::Ld {
            rt: 6,
            ra: 4,
            imm: 0,
        });
        let rest = after_build_high(0x3004, &[ld, word(PpuInstruction::Mtctr { rs: 6 }), bctr()]);
        ordinals(0, &[mtmsrd_stub(msr, &rest), low_window()])
    };
    assert_eq!(census(&msr_ir()), [(LOW_SC, None)]);
    assert_eq!(census(&msr_sf_ir()), [(LOW_SC, Some(75))]);
}

/// The `bctr` of 0x1_0000_3004 is in the second stub. The switch stub
/// ends with `b .`, or with a branch to the second stub.
///
/// [PPC-Book1 p:25 s:2.4] The registers that `bclr` and `bcctr` alter are CTR and LR, so the MSR at the target is the MSR at the branch.
#[test]
fn only_a_bcctr_that_control_reaches_from_a_32_bit_switch_enters_the_low_word() {
    let census = |end: u32| {
        let rest = after_build_high(0x3004, &[word(PpuInstruction::Mtctr { rs: 3 }), bctr()]);
        let segments = [
            mtmsrd_stub(&msr_ir(), &[end]),
            code(OTHER_STUB, &rest),
            low_window(),
        ];
        ordinals(0, &segments)
    };
    let to_other = word(PpuInstruction::B {
        offset: i32::try_from(OTHER_STUB - (STUB + 8)).expect("a near branch"),
        aa: false,
        link: false,
    });
    assert_eq!(census(spin()), [(LOW_SC, Some(75))]);
    assert_eq!(census(to_other), [(LOW_SC, None)]);
}

/// An `rfid` with the SRR1 that `srr1` builds in r4 enters the second
/// stub. With SRR1 = 0 the stub runs in real mode with SF at 0, and its
/// `mtmsr` sets IR to 1 and keeps SF.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` places the OR of SRR1 bits 0 and 1 into MSR\[SF\].
/// [PPC-Book3 p:91 s:8.3] `mtmsr` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\] and leaves MSR bits 0:31 unchanged.
#[test]
fn an_mtmsr_after_a_real_mode_switch_with_sf_at_0_keeps_32_bit_mode() {
    let census = |srr1: u32| {
        let switch = [
            word(PpuInstruction::Addis {
                rt: 3,
                ra: 0,
                imm: 0,
            }),
            word(PpuInstruction::Ori {
                ra: 3,
                rs: 3,
                imm: 0x7000,
            }),
            srr1,
            mtspr(SPR_SRR0, 3),
            mtspr(SPR_SRR1, 4),
            RFID,
        ];
        let mut target = msr_ir();
        target.push(mtmsr(5, false));
        target.extend(after_build_high(
            0x3004,
            &[word(PpuInstruction::Mtctr { rs: 3 }), bctr()],
        ));
        let segments = [code(STUB, &switch), code(OTHER_STUB, &target), low_window()];
        ordinals(0, &segments)
    };
    assert_eq!(census(li(4, 0)), [(LOW_SC, None)]);
    assert_eq!(census(lis_sf(4)), [(LOW_SC, Some(75))]);
}

/// The `rfid` takes SRR0 from a load and SRR1 from r5, which `msr`
/// builds. The `bctr` of 0x1_0000_3004 is in the second stub.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` places the OR of SRR1 bits 0 and 1 into MSR\[SF\].
#[test]
fn an_rfid_to_a_loaded_srr0_with_sf_at_0_lets_every_bcctr_enter_the_low_word() {
    let census = |msr: &[u32]| {
        let mut switch = vec![word(PpuInstruction::Ld {
            rt: 3,
            ra: 4,
            imm: 0,
        })];
        switch.extend_from_slice(msr);
        switch.extend([mtspr(SPR_SRR0, 3), mtspr(SPR_SRR1, 5), RFID]);
        let other = after_build_high(0x3004, &[word(PpuInstruction::Mtctr { rs: 3 }), bctr()]);
        let segments = [code(STUB, &switch), code(OTHER_STUB, &other), low_window()];
        ordinals(0, &segments)
    };
    assert_eq!(census(&msr_ir()), [(LOW_SC, None)]);
    assert_eq!(census(&msr_sf_ir()), [(LOW_SC, Some(75))]);
}

/// A `bl` at 0x1_0000_3000 gives the return point 0x1_0000_3004. No
/// `bclr` follows the `bl`, so no LR constant names that address.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
#[test]
fn a_bclr_in_32_bit_mode_enters_the_low_word_of_a_return_point() {
    let census = |msr: &[u32]| {
        let bl = word(PpuInstruction::B {
            offset: 8,
            aa: false,
            link: true,
        });
        let caller = code(0x1_0000_3000, &[bl, NOP, spin()]);
        ordinals(0, &[mtmsrd_stub(msr, &[PPC_BLR]), caller, low_window()])
    };
    assert_eq!(census(&msr_ir()), [(LOW_SC, None)]);
    assert_eq!(census(&msr_sf_ir()), [(LOW_SC, Some(75))]);
}

/// A `bctr` in 32-bit mode enters the second stub at the low word of
/// 0x1_0000_0000 | `low`, with r5 = 0. At 0x7004 that is the
/// `mtmsrd r5,0`, which then clears IR, so the `ba` reaches the path
/// past the window's `blr` in real mode. That path enters the window's
/// `nop` with r11 = 76. At 0x7000 the stub sets r5 = `MSR_IR` first.
///
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\] and MSR\[SF\] to RS bit 0 | RS bit 1.
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
#[test]
fn a_word_that_a_bcctr_in_32_bit_mode_enters_is_a_join_of_the_switch_scan() {
    let census = |low: u16| {
        let mut rest = vec![li(5, 0)];
        rest.extend(after_build_high(
            low,
            &[word(PpuInstruction::Mtctr { rs: 3 }), bctr()],
        ));
        let ba = word(PpuInstruction::B {
            offset: 0x3010,
            aa: true,
            link: false,
        });
        let mut other = msr_ir();
        other.extend([mtmsrd(5, false), ba]);
        let segments = [
            mtmsrd_stub(&msr_ir(), &rest),
            code(OTHER_STUB, &other),
            window(76),
        ];
        ordinals(0, &segments)
    };
    assert_eq!(census(0x7004), [(SC, None)]);
    assert_eq!(census(0x7000), [(SC, Some(75))]);
}
