use super::hypercall_entry_tests::{code, ordinals, pointers, word, Segment};
use super::hypercall_real_mode_switch_tests::{
    flip_ir, lis_sf, mtspr, site, window, OUTSIDE, RFID, SC,
};
use super::hypercall_tests::NOP;
use super::*;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_FP, MSR_VXU, SPR_SRR0, SPR_SRR1};

/// `mfspr rt,spr`.
///
/// [PPC-Book3 p:21 s:3.4] `mfspr` is primary opcode 31 with extended opcode 339, RT in bits 6:10 and the SPR number in bits 11:20 with its halves swapped.
fn mfspr(rt: u8, spr: u16) -> u32 {
    let spr = u32::from(spr);
    (31 << 26) | (u32::from(rt) << 21) | ((spr & 0x1F) << 16) | ((spr >> 5) << 11) | (339 << 1)
}

/// A stub at [`OUTSIDE`] that runs `words`, then `rfid`.
fn stub(words: &[u32]) -> Segment {
    let mut words = words.to_vec();
    words.push(RFID);
    code(OUTSIDE, &words)
}

/// `mfspr r3,SRR1`, then `change` on r3, then `mtspr SRR1,r3`.
fn rewrite_srr1(change: u32) -> [u32; 3] {
    [mfspr(3, SPR_SRR1), change, mtspr(SPR_SRR1, 3)]
}

/// The census takes SRR0 and SRR1 at the stub's first word to hold what
/// the interrupt saved, and no word writes them before the `rfid`.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0.
#[test]
fn an_rfid_with_srr0_and_srr1_unwritten_since_entry_puts_no_word_in_real_mode() {
    assert_eq!(site(76, stub(&[NOP])), [(SC, Some(75))]);
}

/// The 4.93 lazy floating-point and vector handlers set an enable bit in
/// the saved SRR1 this way and leave MSR\[IR\] and MSR\[PR\] as saved.
///
/// [OFW-4.93 lv2_kernel a:0x8000000000277020-0x8000000000277028] The handler reads SRR1, ORs in 0x0200_0000 and writes SRR1 back.
/// [PPC-Book1 p:66 s:3.3.11] `ori` and `oris` OR RS with UI or UI || 0x0000 and place the result into RA.
/// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
#[test]
fn an_rfid_after_an_srr1_rewrite_that_keeps_ir_and_pr_puts_no_word_in_real_mode() {
    let vxu = u16::try_from(MSR_VXU >> 16).expect("an upper-halfword MSR bit");
    let fp = u16::try_from(MSR_FP).expect("a low MSR bit");
    let changes = [
        word(PpuInstruction::Oris {
            ra: 3,
            rs: 3,
            imm: vxu,
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: fp,
        }),
        word(PpuInstruction::Mr { ra: 3, rs: 3 }),
    ];
    assert_eq!(changes.len(), 3);
    for change in changes {
        let rewritten = stub(&rewrite_srr1(change));
        assert_eq!(site(76, rewritten), [(SC, Some(75))], "{change:#010x}");
    }
}

/// The `xori` flips MSR\[IR\] of the value read back, so SRR1 can clear
/// MSR\[IR\].
#[test]
fn an_rfid_after_an_srr1_rewrite_that_flips_ir_puts_words_in_real_mode() {
    let flipped = || stub(&rewrite_srr1(flip_ir(3)));
    assert_eq!(site(76, flipped()), [(SC, None)]);
    assert_eq!(site(75, flipped()), [(SC, Some(75))]);
}

/// The `or` takes bits of r5, which holds no value from storage at the
/// stub's first word, so SRR1 can hold any MSR\[IR\] and MSR\[PR\].
///
/// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
#[test]
fn an_rfid_after_an_srr1_rewrite_that_ors_in_an_entry_gpr_puts_words_in_real_mode() {
    let or = word(PpuInstruction::Or {
        ra: 3,
        rs: 3,
        rb: 5,
        rc: false,
    });
    let ored = || stub(&rewrite_srr1(or));
    assert_eq!(site(76, ored()), [(SC, None)]);
    assert_eq!(site(75, ored()), [(SC, Some(75))]);
}

/// The copy of SRR0 goes back unchanged, so SRR0 still holds what the
/// interrupt saved.
///
/// [PPC-Book3 p:21 s:3.4] `mfspr` places the SPR that its SPR field names into RT.
/// [PPC-Book3 p:19 s:3.4] `mtspr` copies RS into the SPR that its SPR field names.
#[test]
fn an_rfid_after_an_srr0_read_and_write_back_puts_no_word_in_real_mode() {
    let back = stub(&[mfspr(3, SPR_SRR0), mtspr(SPR_SRR0, 3)]);
    assert_eq!(site(76, back), [(SC, Some(75))]);
}

/// The `oris` sets bit 38 of the copy of SRR0, so SRR0 holds an address
/// that the interrupt did not save.
///
/// [PPC-Book1 p:66 s:3.3.11] `ori` and `oris` OR RS with UI or UI || 0x0000 and place the result into RA.
#[test]
fn an_rfid_after_an_srr0_rewrite_with_oris_puts_words_in_real_mode() {
    let oris = word(PpuInstruction::Oris {
        ra: 3,
        rs: 3,
        imm: 0x0200,
    });
    let rewritten = || stub(&[mfspr(3, SPR_SRR0), oris, mtspr(SPR_SRR0, 3)]);
    assert_eq!(site(76, rewritten()), [(SC, None)]);
    assert_eq!(site(75, rewritten()), [(SC, Some(75))]);
}

/// A code pointer names the `nop`, which makes it a join. The loads
/// before the join give r3 and r4 on the fall-through path only, so the
/// moves after the join write SRR0 and SRR1 with values that the census
/// does not take as the interrupted context.
///
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads a doubleword from storage into RT.
/// [PPC-Book3 p:19 s:3.4] `mtspr` copies RS into the SPR that its SPR field names.
#[test]
fn a_join_forgets_the_loads_of_the_fall_through_before_it() {
    let ld = |rt, imm| word(PpuInstruction::Ld { rt, ra: 1, imm });
    let moved = || {
        stub(&[
            ld(3, 0),
            ld(4, 8),
            NOP,
            mtspr(SPR_SRR0, 3),
            mtspr(SPR_SRR1, 4),
        ])
    };
    let with_join = |other| {
        ordinals(
            0,
            &[moved(), window(other), pointers(&[OUTSIDE, OUTSIDE + 8])],
        )
    };
    assert_eq!(with_join(76), [(SC, None)]);
    assert_eq!(with_join(75), [(SC, Some(75))]);
}

/// r3 holds no value from storage at the stub's first word, so the move
/// writes SRR0 with a value that the census does not take as the
/// interrupted context.
///
/// [PPC-Book3 p:19 s:3.4] `mtspr` copies RS into the SPR that its SPR field names.
#[test]
fn an_rfid_after_an_srr0_write_of_an_entry_gpr_puts_words_in_real_mode() {
    let written = || stub(&[mtspr(SPR_SRR0, 3)]);
    assert_eq!(site(76, written()), [(SC, None)]);
    assert_eq!(site(75, written()), [(SC, Some(75))]);
}

/// The copy of SRR0 holds an address, so the move into SRR1 writes an
/// SRR1 whose MSR\[IR\] and MSR\[PR\] are bits of that address. The copy of
/// SRR1 moved into SRR0 likewise writes SRR0 with an MSR value.
///
/// [PPC-Book3 p:21 s:3.4] `mfspr` places the SPR that its SPR field names into RT.
/// [PPC-Book3 p:19 s:3.4] `mtspr` copies RS into the SPR that its SPR field names.
#[test]
fn an_rfid_after_a_copy_of_one_srr_moved_into_the_other_puts_words_in_real_mode() {
    let crossed = [
        [mfspr(3, SPR_SRR0), mtspr(SPR_SRR1, 3)],
        [mfspr(3, SPR_SRR1), mtspr(SPR_SRR0, 3)],
    ];
    assert_eq!(crossed.len(), 2);
    for words in crossed {
        assert_eq!(site(76, stub(&words)), [(SC, None)], "{words:x?}");
        assert_eq!(site(75, stub(&words)), [(SC, Some(75))], "{words:x?}");
    }
}

/// The routine of the call can write SRR0 and SRR1. The word after the
/// call is a join, and the join keeps what the call left.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
#[test]
fn a_call_before_the_rfid_forgets_the_saved_srrs() {
    let bl = word(PpuInstruction::B {
        offset: 4,
        aa: false,
        link: true,
    });
    let after_call = || stub(&[bl]);
    assert_eq!(site(76, after_call()), [(SC, None)]);
    assert_eq!(site(75, after_call()), [(SC, Some(75))]);
}

/// The word does not decode, so the interrupt that it causes sets SRR0
/// and SRR1 before control can reach the `rfid`.
///
/// [PPC-Book3 p:71 s:5.5.9] An Illegal Instruction type Program interrupt sets SRR0 to the effective address of the instruction that caused it.
#[test]
fn a_word_that_does_not_decode_before_the_rfid_forgets_the_saved_srrs() {
    let undecoded = || stub(&[0]);
    assert_eq!(site(76, undecoded()), [(SC, None)]);
    assert_eq!(site(75, undecoded()), [(SC, Some(75))]);
}

/// A code pointer names the `nop`, which makes it a join. The fall-through
/// path into the join wrote SRR1 with MSR\[IR\] at 0, and the join keeps
/// that write.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0.
#[test]
fn a_join_that_a_fall_through_reaches_keeps_the_srr1_write_before_it() {
    let written = || stub(&[lis_sf(4), mtspr(SPR_SRR1, 4), NOP]);
    let with_join = |other| {
        ordinals(
            0,
            &[written(), window(other), pointers(&[OUTSIDE, OUTSIDE + 8])],
        )
    };
    assert_eq!(with_join(76), [(SC, None)]);
    assert_eq!(with_join(75), [(SC, Some(75))]);
}
