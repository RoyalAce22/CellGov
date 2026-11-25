use super::hypercall_entry_tests::{code, word, Segment};
use super::hypercall_real_mode_switch_tests::{flip_ir, mtspr, site, OUTSIDE, RFID, SC};
use super::hypercall_tests::li;
use super::*;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_FP, MSR_IR, MSR_PR, SPR_SRR0, SPR_SRR1};

/// `ld rt,imm(r1)`.
fn ld(rt: u8, imm: i16) -> u32 {
    word(PpuInstruction::Ld { rt, ra: 1, imm })
}

/// A stub at [`OUTSIDE`] that builds SRR0 in r3 with `srr0` and SRR1 in
/// r4 with `srr1`, moves both into place, runs `between`, then runs
/// `rfid`.
fn restore(srr0: &[u32], srr1: &[u32], between: &[u32]) -> Segment {
    let mut words = srr0.to_vec();
    words.extend_from_slice(srr1);
    words.push(mtspr(SPR_SRR0, 3));
    words.push(mtspr(SPR_SRR1, 4));
    words.extend_from_slice(between);
    words.push(RFID);
    code(OUTSIDE, &words)
}

/// Each doubleword load and each word load that fills the high word with
/// 0, into `rt` from the frame at r1.
fn loads(rt: u8) -> [u32; 8] {
    let (ra, rb, imm) = (1, 5, 8);
    [
        word(PpuInstruction::Ld { rt, ra, imm }),
        word(PpuInstruction::Ldx { rt, ra, rb }),
        word(PpuInstruction::Ldu { rt, ra, imm }),
        word(PpuInstruction::Ldux { rt, ra, rb }),
        word(PpuInstruction::Lwz { rt, ra, imm }),
        word(PpuInstruction::Lwzx { rt, ra, rb }),
        word(PpuInstruction::Lwzu { rt, ra, imm }),
        word(PpuInstruction::Lwzux { rt, ra, rb }),
    ]
}

/// The census takes such an `rfid` to resume an interrupted word in the
/// mode of that word, as it takes an interrupt and its return.
///
/// [PPC-Book1 p:37 s:3.3.2] `lwz`, `lwzx`, `lwzu` and `lwzux` load a word from storage into RT bits 32:63 and set RT bits 0:31 to 0.
/// [PPC-Book1 p:39 s:3.3.2] `ld`, `ldx`, `ldu` and `ldux` load a doubleword from storage into RT.
#[test]
fn an_rfid_whose_srr0_and_srr1_are_loaded_from_storage_puts_no_word_in_real_mode() {
    let pairs: Vec<(u32, u32)> = loads(3).into_iter().zip(loads(4)).collect();
    assert_eq!(pairs.len(), 8);
    for (srr0, srr1) in pairs {
        let stub = restore(&[srr0], &[srr1], &[]);
        assert_eq!(site(76, stub), [(SC, Some(75))], "{srr0:#010x}");
    }
}

/// The `xori` flips IR of r4 after the load, so SRR1 holds an IR that the
/// census does not take from storage, and that IR can be 0.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0.
#[test]
fn an_rfid_whose_loaded_srr1_is_changed_before_the_move_puts_words_in_real_mode() {
    let changed = || restore(&[ld(3, 0)], &[ld(4, 8), flip_ir(4)], &[]);
    assert_eq!(site(76, changed()), [(SC, None)]);
    assert_eq!(site(75, changed()), [(SC, Some(75))]);
}

/// `andc r4,r4,r5` after `li r5,bits`.
fn clear(bits: u64) -> [u32; 2] {
    let bits = i16::try_from(bits).expect("a low MSR bit");
    let andc = word(PpuInstruction::Andc {
        ra: 4,
        rs: 4,
        rb: 5,
        rc: false,
    });
    [li(5, bits), andc]
}

/// `lwz r13,16(r1)`, then `rlwimi r4,r13,0,bit-32,bit-32`, which puts
/// a bit of the second load into bit `bit` of r4.
fn insert(bit: u8) -> [u32; 2] {
    let lwz = word(PpuInstruction::Lwz {
        rt: 13,
        ra: 1,
        imm: 16,
    });
    let field = bit - 32;
    let rlwimi = word(PpuInstruction::Rlwimi {
        ra: 4,
        rs: 13,
        sh: 0,
        mb: field,
        me: field,
        rc: false,
    });
    [lwz, rlwimi]
}

/// MSR\[FP\] and MSR bit 38 are bits outside MSR\[IR\] and MSR\[PR\], so
/// SRR1 still takes the mode bits from storage.
///
/// [PPC-Book1 p:68 s:3.3.11] `andc` ANDs RS with the complement of RB into RA.
/// [PPC-Book1 p:76 s:3.3.12.1] `rlwimi` inserts the rotated RS into RA under MASK(MB + 32, ME + 32) and leaves the other bits of RA unchanged.
#[test]
fn an_rfid_whose_loaded_srr1_changes_only_bits_outside_ir_and_pr_puts_no_word_in_real_mode() {
    let cleared = restore(
        &[ld(3, 0)],
        &[&[ld(4, 8)][..], &clear(MSR_FP)].concat(),
        &[],
    );
    assert_eq!(site(76, cleared), [(SC, Some(75))]);
    let inserted = restore(&[ld(3, 0)], &[&[ld(4, 8)][..], &insert(38)].concat(), &[]);
    assert_eq!(site(76, inserted), [(SC, Some(75))]);
}

/// MSR\[IR\] is bit 58, so the `andc` and the `rlwimi` each give SRR1 an
/// IR that the census does not take from storage.
///
/// [PPC-Book1 p:68 s:3.3.11] `andc` ANDs RS with the complement of RB into RA.
/// [PPC-Book1 p:76 s:3.3.12.1] `rlwimi` inserts the rotated RS into RA under MASK(MB + 32, ME + 32) and leaves the other bits of RA unchanged.
#[test]
fn an_rfid_whose_loaded_srr1_changes_ir_puts_words_in_real_mode() {
    let cleared = || {
        restore(
            &[ld(3, 0)],
            &[&[ld(4, 8)][..], &clear(MSR_IR)].concat(),
            &[],
        )
    };
    assert_eq!(site(76, cleared()), [(SC, None)]);
    assert_eq!(site(75, cleared()), [(SC, Some(75))]);
    let inserted = restore(&[ld(3, 0)], &[&[ld(4, 8)][..], &insert(58)].concat(), &[]);
    assert_eq!(site(76, inserted), [(SC, None)]);
}

/// MSR\[PR\] is bit 49, so the `andc` and the `rlwimi` each give SRR1 a PR
/// that the census does not take from storage. With PR at 0 the loaded IR
/// alone sets MSR\[IR\], and that IR can be 0.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0.
/// [PPC-Book1 p:68 s:3.3.11] `andc` ANDs RS with the complement of RB into RA.
/// [PPC-Book1 p:76 s:3.3.12.1] `rlwimi` inserts the rotated RS into RA under MASK(MB + 32, ME + 32) and leaves the other bits of RA unchanged.
#[test]
fn an_rfid_whose_loaded_srr1_changes_pr_puts_words_in_real_mode() {
    let cleared = || {
        restore(
            &[ld(3, 0)],
            &[&[ld(4, 8)][..], &clear(MSR_PR)].concat(),
            &[],
        )
    };
    assert_eq!(site(76, cleared()), [(SC, None)]);
    assert_eq!(site(75, cleared()), [(SC, Some(75))]);
    let inserted = restore(&[ld(3, 0)], &[&[ld(4, 8)][..], &insert(49)].concat(), &[]);
    assert_eq!(site(76, inserted), [(SC, None)]);
}

/// `lwz r13,16(r1)`, then `rlwimi r4,r13,0,mb,me` with `mb` > `me`.
fn insert_wrapped(mb: u8, me: u8) -> [u32; 2] {
    let lwz = word(PpuInstruction::Lwz {
        rt: 13,
        ra: 1,
        imm: 16,
    });
    let rlwimi = word(PpuInstruction::Rlwimi {
        ra: 4,
        rs: 13,
        sh: 0,
        mb,
        me,
        rc: false,
    });
    [lwz, rlwimi]
}

/// With MB > ME the mask wraps: MASK(58, 32) holds bits 58:63 and 0:32,
/// which hold MSR\[IR\], and MASK(63, 42) holds bit 63 and bits 0:42,
/// which hold neither MSR\[IR\] (bit 58) nor MSR\[PR\] (bit 49).
///
/// [PPC-Book1 p:76 s:3.3.12.1] `rlwimi` inserts the rotated RS into RA under MASK(MB + 32, ME + 32) and leaves the other bits of RA unchanged.
#[test]
fn an_rfid_after_a_wrapped_rlwimi_into_the_loaded_srr1_follows_the_wrapped_mask() {
    let with = |mb, me| {
        restore(
            &[ld(3, 0)],
            &[&[ld(4, 8)][..], &insert_wrapped(mb, me)].concat(),
            &[],
        )
    };
    assert_eq!(site(76, with(26, 0)), [(SC, None)]);
    assert_eq!(site(75, with(26, 0)), [(SC, Some(75))]);
    assert_eq!(site(76, with(31, 10)), [(SC, Some(75))]);
}

/// The scan keeps no constant in r5, so the `andc` can clear any bit of
/// the loaded SRR1.
#[test]
fn an_rfid_whose_loaded_srr1_is_masked_by_a_loaded_value_puts_words_in_real_mode() {
    let andc = word(PpuInstruction::Andc {
        ra: 4,
        rs: 4,
        rb: 5,
        rc: false,
    });
    let masked = || restore(&[ld(3, 0)], &[ld(4, 8), ld(5, 16), andc], &[]);
    assert_eq!(site(76, masked()), [(SC, None)]);
    assert_eq!(site(75, masked()), [(SC, Some(75))]);
}

/// [PPC-Book3 p:73 s:5.5.13] A System Call interrupt sets SRR0 and SRR1.
#[test]
fn an_sc_between_the_restore_and_the_rfid_forgets_the_loaded_srrs() {
    let sc = word(PpuInstruction::Sc { lev: 0 });
    let after_sc = || restore(&[ld(3, 0)], &[ld(4, 8)], &[sc]);
    assert_eq!(site(76, after_sc()), [(SC, None)]);
    assert_eq!(site(75, after_sc()), [(SC, Some(75))]);
}
