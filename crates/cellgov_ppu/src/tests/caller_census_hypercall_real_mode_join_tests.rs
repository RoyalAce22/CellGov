use super::hypercall_entry_tests::{code, data_bytes, ordinals, word, Segment};
use super::hypercall_real_mode_switch_tests::{
    build, lis_sf, msr_stub, mtmsr, mtmsrd, mtspr, ori, shift_left, site, window, OUTSIDE, PAST,
    RFID, SC,
};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_IR, SPR_SRR0, SPR_SRR1};

/// `ld rt,0(r1)`: a value that the scan does not know.
fn ld(rt: u8) -> u32 {
    word(PpuInstruction::Ld { rt, ra: 1, imm: 0 })
}

/// `b` at `from` to `to`.
fn b_to(from: u64, to: u64) -> u32 {
    word(PpuInstruction::B {
        offset: i32::try_from(to.wrapping_sub(from).cast_signed()).expect("a near branch"),
        aa: false,
        link: false,
    })
}

/// `bl` with the displacement `offset`.
fn bl(offset: i32) -> u32 {
    word(PpuInstruction::B {
        offset,
        aa: false,
        link: true,
    })
}

/// `lis r4,-0x8000; ori r4,r4,MSR_IR`: an SRR1 with IR at 1.
fn ir_set() -> [u32; 2] {
    [lis_sf(4), ori(4, MSR_IR)]
}

/// A stub at [`OUTSIDE`] that writes r3 into SRR0 and r4 into SRR1 after
/// `before`, then runs `rfid`.
fn rfid_after(before: &[u32]) -> Segment {
    let mut words = before.to_vec();
    words.extend([mtspr(SPR_SRR0, 3), mtspr(SPR_SRR1, 4), RFID]);
    code(OUTSIDE, &words)
}

/// The address of word 3 of [`late_srr0`].
const LATE: u64 = OUTSIDE + 12;

/// A stub at [`OUTSIDE`] that writes SRR1 with IR at 1, then from word 3
/// writes [`PAST`] into SRR0 and runs `rfid`, with `after` past the
/// `rfid`.
fn late_srr0(after: &[u32]) -> Segment {
    let mut words = ir_set().to_vec();
    words.push(mtspr(SPR_SRR1, 4));
    words.extend(build(3, PAST));
    words.extend([mtspr(SPR_SRR0, 3), RFID]);
    words.extend_from_slice(after);
    code(OUTSIDE, &words)
}

/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0.
#[test]
fn an_rfid_that_a_branch_reaches_after_the_srr_writes_can_put_the_window_in_real_mode() {
    let stub = || {
        let mut words = build(3, PAST).to_vec();
        words.extend([lis_sf(4), mtspr(SPR_SRR0, 3), mtspr(SPR_SRR1, 4)]);
        words.push(b_to(OUTSIDE + 20, OUTSIDE + 24));
        words.push(RFID);
        code(OUTSIDE, &words)
    };
    assert_eq!(site(76, stub()), [(SC, None)]);
    assert_eq!(site(75, stub()), [(SC, Some(75))]);
}

/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0.
#[test]
fn an_rfid_with_a_loaded_srr1_puts_only_its_built_srr0_in_real_mode() {
    let to = |srr0: u16| rfid_after(&[build(3, srr0)[0], build(3, srr0)[1], ld(4)]);
    assert_eq!(site(76, to(PAST)), [(SC, None)]);
    assert_eq!(site(76, to(0x6000)), [(SC, Some(75))]);
}

/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0.
#[test]
fn an_rfid_with_a_loaded_srr0_and_srr1_can_put_the_window_in_real_mode() {
    let stub = || rfid_after(&[ld(3), ld(4)]);
    assert_eq!(site(76, stub()), [(SC, None)]);
    assert_eq!(site(75, stub()), [(SC, Some(75))]);
}

/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\].
#[test]
fn an_rfid_with_a_loaded_srr0_and_a_built_srr1_that_sets_ir_adds_no_path() {
    let [lis, ori_ir] = ir_set();
    assert_eq!(
        site(76, rfid_after(&[ld(3), lis, ori_ir])),
        [(SC, Some(75))]
    );
}

/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\], and with L = 1 sets only MSR bits 48 and 62.
/// [PPC-Book3 p:91 s:8.3] `mtmsr` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\].
#[test]
fn an_mtmsr_or_mtmsrd_with_a_loaded_rs_puts_its_successor_in_real_mode_only_with_l_0() {
    for set in [mtmsrd(5, false), mtmsr(5, false)] {
        assert_eq!(site(76, msr_stub(&[ld(5)], set)), [(SC, None)], "{set:#x}");
        assert_eq!(
            site(75, msr_stub(&[ld(5)], set)),
            [(SC, Some(75))],
            "{set:#x}"
        );
    }
    for set in [mtmsrd(5, true), mtmsr(5, true)] {
        assert_eq!(
            site(76, msr_stub(&[ld(5)], set)),
            [(SC, Some(75))],
            "{set:#x}"
        );
    }
}

#[test]
fn a_word_between_the_srr1_write_and_rfid_that_a_branch_reaches_forgets_srr1() {
    assert_eq!(site(76, late_srr0(&[])), [(SC, Some(75))]);
    let back = b_to(OUTSIDE + 28, LATE);
    assert_eq!(site(76, late_srr0(&[back])), [(SC, None)]);
}

/// The image names word 0 of the stub, before the SRR1 write, or word 3.
#[test]
fn a_word_between_the_srr1_write_and_rfid_that_the_image_names_forgets_srr1() {
    let named = |address: u64| data_bytes(KERNEL + 0x8_0000, address.to_be_bytes().to_vec());
    let before = [late_srr0(&[]), named(OUTSIDE), window(76)];
    assert_eq!(ordinals(0, &before), [(SC, Some(75))]);
    let between = [late_srr0(&[]), named(LATE), window(76)];
    assert_eq!(ordinals(0, &between), [(SC, None)]);
}

/// The `bcctr` targets the real address of word 0 of the stub, before
/// the SRR1 write, or of word 3.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_word_between_the_srr1_write_and_rfid_at_the_real_address_of_a_bcctr_target_forgets_srr1() {
    let bctr = word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    });
    let to = |address: u64| {
        let real = i16::try_from(address & 0xFFFF).expect("a low real address");
        code(
            KERNEL + 0x7000,
            &[li(12, real), word(PpuInstruction::Mtctr { rs: 12 }), bctr],
        )
    };
    let before = [late_srr0(&[]), to(OUTSIDE), window(76)];
    assert_eq!(ordinals(0, &before), [(SC, Some(75))]);
    let between = [late_srr0(&[]), to(LATE), window(76)];
    assert_eq!(ordinals(0, &between), [(SC, None)]);
}

#[test]
fn a_call_between_the_msr_build_and_mtmsrd_forgets_rs_and_a_link_to_the_next_word_keeps_it() {
    let [lis, ori_ir] = [lis_sf(5), ori(5, MSR_IR)];
    let after = |call| msr_stub(&[lis, ori_ir, call], mtmsrd(5, false));
    assert_eq!(site(76, after(bl(0x100))), [(SC, None)]);
    assert_eq!(site(76, after(bl(4))), [(SC, Some(75))]);
}

/// The first `rfid` enters word 0 or word 1 of the second stub at the
/// low word of its SRR0. Word 1 is a join that only a switch target
/// makes. Word 0 builds SRR1 itself.
///
/// [PPC-Book3 p:13 s:2.3] With SF = 0 in the new MSR, `rfid` fetches from 32 zeros || SRR0 32:61 || 0b00.
#[test]
fn a_switch_target_forgets_the_srr1_that_the_path_before_it_builds() {
    let second = KERNEL + 0x6000;
    let first = |to: u64| {
        let low = u16::try_from(to & 0xFFFF).expect("a low word");
        rfid_after(&[
            li(3, 1),
            shift_left(3, 32),
            ori(3, u64::from(low)),
            li(4, 0),
        ])
    };
    let mut words = vec![li(4, i16::try_from(MSR_IR).expect("a low MSR bit"))];
    words.extend(build(3, PAST));
    words.extend([mtspr(SPR_SRR0, 3), mtspr(SPR_SRR1, 4), RFID]);
    let to_word_0 = [first(second), code(second, &words), window(76)];
    assert_eq!(ordinals(0, &to_word_0), [(SC, Some(75))]);
    let to_word_1 = [first(second + 4), code(second, &words), window(76)];
    assert_eq!(ordinals(0, &to_word_1), [(SC, None)]);
}

/// The window lies at real address 0x3000, and a word of another segment
/// shares the real address of its middle word, which the image names.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn an_unbounded_rfid_enters_the_real_address_of_each_named_word() {
    let site = |srr1: &[u32]| {
        let mut before = vec![ld(3)];
        before.extend_from_slice(srr1);
        ordinals(
            0,
            &[
                rfid_after(&before),
                code(0x3000, &[li(11, 75), NOP, PPC_SC_1, PPC_BLR]),
                code(KERNEL + 0x3000, &[NOP, NOP]),
                data_bytes(KERNEL + 0x8_0000, (KERNEL + 0x3004).to_be_bytes().to_vec()),
            ],
        )
    };
    assert_eq!(site(&ir_set()), [(0x3008, Some(75))]);
    assert_eq!(site(&[ld(4)]), [(0x3008, None)]);
}
