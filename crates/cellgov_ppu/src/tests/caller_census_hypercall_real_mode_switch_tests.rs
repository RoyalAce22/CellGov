use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_BIT1, MSR_IR, MSR_PR, MSR_SF, SPR_SRR0, SPR_SRR1};

/// A word outside the interrupt vectors, where no real-mode path goes.
pub(super) const OUTSIDE: u64 = KERNEL + 0x5000;

/// The kernel address of the segment that [`window`] gives.
pub(super) const WINDOW: u64 = KERNEL + 0x3000;

/// The kernel address of the window's `sc`.
pub(super) const SC: u64 = WINDOW + 8;

/// The real address of the path past the window's `blr`.
pub(super) const PAST: u16 = 0x3010;

/// A window at [`WINDOW`] that loads 75 and runs `sc`, and past its `blr`
/// a path that loads `other` and branches to the real address of the
/// word after the window's `li r11`.
pub(super) fn window(other: i16) -> Segment {
    let ba = word(PpuInstruction::B {
        offset: 0x3004,
        aa: true,
        link: false,
    });
    code(
        WINDOW,
        &[li(11, 75), NOP, PPC_SC_1, PPC_BLR, li(11, other), ba],
    )
}

/// The window's site, with `other` in the window and `stub` as one more
/// segment.
pub(super) fn site(other: i16, stub: Segment) -> Vec<(u64, Option<u16>)> {
    ordinals(0, &[stub, window(other)])
}

/// `lis rt,0; ori rt,rt,address`.
pub(super) fn build(rt: u8, address: u16) -> [u32; 2] {
    [
        word(PpuInstruction::Addis { rt, ra: 0, imm: 0 }),
        word(PpuInstruction::Ori {
            ra: rt,
            rs: rt,
            imm: address,
        }),
    ]
}

/// `lis rt,-0x8000`, which builds 0xFFFF_FFFF_8000_0000: SF = 1, and IR
/// and PR at 0.
pub(super) fn lis_sf(rt: u8) -> u32 {
    word(PpuInstruction::Addis {
        rt,
        ra: 0,
        imm: i16::MIN,
    })
}

/// `ori rt,rt,bits`.
pub(super) fn ori(rt: u8, bits: u64) -> u32 {
    word(PpuInstruction::Ori {
        ra: rt,
        rs: rt,
        imm: u16::try_from(bits).expect("a low MSR bit"),
    })
}

/// `mtspr spr,rs`.
///
/// [PPC-Book3 p:19 s:3.4] `mtspr` is primary opcode 31 with extended opcode 467, RS in bits 6:10 and the SPR number in bits 11:20 with its halves swapped.
pub(super) fn mtspr(spr: u16, rs: u8) -> u32 {
    let spr = u32::from(spr);
    (31 << 26) | (u32::from(rs) << 21) | ((spr & 0x1F) << 16) | ((spr >> 5) << 11) | (467 << 1)
}

/// `mtmsrd rs,l`.
///
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` is primary opcode 31 with extended opcode 178, RS in bits 6:10 and L in bit 15.
pub(super) fn mtmsrd(rs: u8, l: bool) -> u32 {
    (31 << 26) | (u32::from(rs) << 21) | (u32::from(l) << 16) | (178 << 1)
}

/// `mtmsr rs,l`.
///
/// [PPC-Book3 p:91 s:8.3] `mtmsr` is primary opcode 31 with extended opcode 146, RS in bits 6:10 and L in bit 15.
pub(super) fn mtmsr(rs: u8, l: bool) -> u32 {
    (31 << 26) | (u32::from(rs) << 21) | (u32::from(l) << 16) | (146 << 1)
}

/// `rldicr rt,rt,sh,63-sh`, which shifts rt left by `sh` bits.
pub(super) fn shift_left(rt: u8, sh: u8) -> u32 {
    word(PpuInstruction::Rldicr {
        ra: rt,
        rs: rt,
        sh,
        me: 63 - sh,
        rc: false,
    })
}

/// [PPC-Book3 p:119 s:Appendix G] `rfid` is an XL-form word with primary opcode 19 and extended opcode 18.
pub(super) const RFID: u32 = (19 << 26) | (18 << 1);

/// A stub at [`OUTSIDE`] that gives SRR0 [`PAST`] and SRR1 the value
/// that `srr1` builds in r4, with `between` after the SRR1 write, then
/// runs `rfid`.
pub(super) fn rfid_stub(srr1: &[u32], between: &[u32]) -> Segment {
    let mut words = build(3, PAST).to_vec();
    words.extend_from_slice(srr1);
    words.push(mtspr(SPR_SRR0, 3));
    words.push(mtspr(SPR_SRR1, 4));
    words.extend_from_slice(between);
    words.push(RFID);
    code(OUTSIDE, &words)
}

/// A stub at [`OUTSIDE`] that runs `set` with the value that `msr`
/// builds in r5, then branches to the path past the window's `blr`.
pub(super) fn msr_stub(msr: &[u32], set: u32) -> Segment {
    let mut words = msr.to_vec();
    words.push(set);
    let at = OUTSIDE + 4 * words.len() as u64;
    let to_past = WINDOW + 0x10;
    words.push(word(PpuInstruction::B {
        offset: i32::try_from(to_past.wrapping_sub(at).cast_signed()).expect("a near branch"),
        aa: false,
        link: false,
    }));
    code(OUTSIDE, &words)
}

/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0.
#[test]
fn an_rfid_with_a_built_srr1_that_clears_ir_puts_its_target_in_real_mode() {
    let stub = || rfid_stub(&[lis_sf(4)], &[]);
    assert_eq!(site(76, stub()), [(SC, None)]);
    assert_eq!(site(75, stub()), [(SC, Some(75))]);
}

/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\].
#[test]
fn an_mtmsrd_that_clears_ir_puts_its_successor_in_real_mode() {
    let stub = || msr_stub(&[lis_sf(5)], mtmsrd(5, false));
    assert_eq!(site(76, stub()), [(SC, None)]);
    assert_eq!(site(75, stub()), [(SC, Some(75))]);
}

/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\].
#[test]
fn an_rfid_whose_built_srr1_sets_ir_or_pr_adds_no_path() {
    for bit in [MSR_IR, MSR_PR] {
        let stub = rfid_stub(&[lis_sf(4), ori(4, bit)], &[]);
        assert_eq!(site(76, stub), [(SC, Some(75))], "{bit:#x}");
    }
}

/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\], so a loaded SRR1 can clear IR.
#[test]
fn an_rfid_whose_srr1_is_loaded_puts_its_target_in_real_mode() {
    let ld = word(PpuInstruction::Ld {
        rt: 4,
        ra: 1,
        imm: 0,
    });
    assert_eq!(site(76, rfid_stub(&[ld], &[])), [(SC, None)]);
    assert_eq!(site(75, rfid_stub(&[ld], &[])), [(SC, Some(75))]);
}

/// [PPC-Book3 p:73 s:5.5.13] A System Call interrupt sets SRR0 and SRR1.
#[test]
fn an_sc_between_the_srr1_write_and_rfid_forgets_srr1() {
    let sc = word(PpuInstruction::Sc { lev: 0 });
    let ir = [lis_sf(4), ori(4, MSR_IR)];
    assert_eq!(site(76, rfid_stub(&ir, &[])), [(SC, Some(75))]);
    assert_eq!(site(76, rfid_stub(&ir, &[sc])), [(SC, None)]);
}

#[test]
fn a_bl_to_the_next_word_between_the_srr1_write_and_rfid_forgets_srr1() {
    let bl = word(PpuInstruction::B {
        offset: 4,
        aa: false,
        link: true,
    });
    let ir = [lis_sf(4), ori(4, MSR_IR)];
    assert_eq!(site(76, rfid_stub(&ir, &[])), [(SC, Some(75))]);
    assert_eq!(site(76, rfid_stub(&ir, &[bl])), [(SC, None)]);
}

/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 1 sets only MSR bits 48 and 62.
#[test]
fn an_mtmsrd_with_l_1_adds_no_path() {
    let stub = msr_stub(&[lis_sf(5)], mtmsrd(5, true));
    assert_eq!(site(76, stub), [(SC, Some(75))]);
}

/// [PPC-Book3 p:91 s:8.3] `mtmsr` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\], and with L = 1 sets only MSR bits 48 and 62.
#[test]
fn an_mtmsr_that_clears_ir_puts_its_successor_in_real_mode_only_with_l_0() {
    let stub = |l| msr_stub(&[li(5, 0)], mtmsr(5, l));
    assert_eq!(site(76, stub(false)), [(SC, None)]);
    assert_eq!(site(75, stub(false)), [(SC, Some(75))]);
    assert_eq!(site(76, stub(true)), [(SC, Some(75))]);
}

/// An address whose high word is not 0, where code runs with SF = 1.
///
/// [PPC-Book1 p:15 s:1.12] In 32-bit mode the high-order 32 bits of the effective address are effectively set to 0 for the purpose of fetching instructions.
const HIGH_WORD: u64 = 0x0000_0001_0000_5000;

/// A stub at [`HIGH_WORD`] gives r5 the value 0 and runs `set`. The word
/// after `set` is at 0x1_0000_5008. The low word of that address is the
/// real address of [`OUTSIDE`] + 8, where a `b` goes to the path past
/// the window's `blr`.
///
/// [PPC-Book3 p:91 s:8.3] `mtmsr` with L = 0 changes no MSR bit in 0:31, so MSR\[SF\] keeps its value.
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 places the OR of RS bits 0 and 1 into MSR\[SF\].
/// [PPC-Book1 p:15 s:1.12] In 32-bit mode the high-order 32 bits of the effective address are effectively set to 0 for the purpose of fetching instructions.
#[test]
fn an_mtmsr_keeps_the_high_word_of_its_successor_and_an_mtmsrd_with_sf_0_drops_it() {
    let low = OUTSIDE + 8;
    let to_past = WINDOW + 0x10;
    let b = word(PpuInstruction::B {
        offset: i32::try_from(to_past.wrapping_sub(low).cast_signed()).expect("a near branch"),
        aa: false,
        link: false,
    });
    let census = |set: u32| {
        let stub = code(HIGH_WORD, &[li(5, 0), set, NOP]);
        ordinals(0, &[stub, code(low, &[b]), window(76)])
    };
    assert_eq!(census(mtmsr(5, false)), [(SC, Some(75))]);
    assert_eq!(census(mtmsrd(5, false)), [(SC, None)]);
}

/// SRR0 0x1_0000_3010 reaches no word of the code when the new MSR has
/// SF = 1.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` places the OR of SRR1 bits 0 and 1 into MSR\[SF\], and with SF = 1 in the new MSR fetches from SRR0 0:61 || 0b00.
#[test]
fn an_rfid_with_srr1_bit_0_or_bit_1_keeps_the_high_word_of_srr0() {
    for bit in [MSR_SF, MSR_BIT1] {
        let sh = u8::try_from(bit.trailing_zeros()).expect("a doubleword bit");
        let words = [
            li(3, 1),
            shift_left(3, 32),
            ori(3, u64::from(PAST)),
            li(4, 1),
            shift_left(4, sh),
            mtspr(SPR_SRR0, 3),
            mtspr(SPR_SRR1, 4),
            RFID,
        ];
        assert_eq!(
            site(76, code(OUTSIDE, &words)),
            [(SC, Some(75))],
            "{bit:#x}"
        );
    }
}

/// SRR0 0x1_0000_3010 reaches [`PAST`] only when the new MSR has SF = 0.
///
/// [PPC-Book3 p:13 s:2.3] With SF = 0 in the new MSR, `rfid` fetches from 32 zeros || SRR0 32:61 || 0b00.
#[test]
fn an_rfid_with_sf_0_reaches_the_low_word_of_srr0() {
    let one = li(3, 1);
    let sldi = word(PpuInstruction::Rldicr {
        ra: 3,
        rs: 3,
        sh: 32,
        me: 31,
        rc: false,
    });
    let low = ori(3, u64::from(PAST));
    let words = [
        one,
        sldi,
        low,
        li(4, 0),
        mtspr(SPR_SRR0, 3),
        mtspr(SPR_SRR1, 4),
        RFID,
    ];
    assert_eq!(site(76, code(OUTSIDE, &words)), [(SC, None)]);
}
