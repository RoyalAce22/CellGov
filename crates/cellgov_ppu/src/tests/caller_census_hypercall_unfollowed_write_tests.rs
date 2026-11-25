use super::hypercall_constant_join_tests::{
    addi, bctr, descriptor, mtctr, sites, skip, twenty_pass_loop, DESCRIPTORS, EVERY_UNRESOLVED,
    NOWHERE, WINDOW_RESOLVED,
};
use super::hypercall_ctr_copy_tests::mfctr;
use super::hypercall_ctr_decrement_tests::{bc, quiet, window, SC, VECTOR};
use super::hypercall_entry_tests::{code, data_bytes, elf, ordinals, word};
use super::hypercall_join_drop_tests::RAISED_LIMIT;
use super::hypercall_real_mode_switch_tests::{mtmsrd, mtspr};
use super::hypercall_skip_return_tests::{mflr, mtlr, ordinals_with_routine};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;
use cellgov_ps3_abi::hw::ppu_supervisor::{SPR_SPRG0, SPR_SRR0};

/// A word past the interrupt vectors, 0x7000 past the kernel base. No
/// constant of the cases names a word after it.
const AWAY: u64 = VECTOR + 0x6F00;

/// `mfspr rt,spr`.
///
/// [PPC-Book3 p:21 s:3.4] `mfspr` is primary opcode 31 with extended opcode 339, RT in bits 6:10 and the SPR number in bits 11:20 with its halves swapped.
fn mfspr(rt: u8, spr: u16) -> u32 {
    let spr = u32::from(spr);
    (31 << 26) | (u32::from(rt) << 21) | ((spr & 0x1F) << 16) | ((spr >> 5) << 11) | (339 << 1)
}

/// `add rt,ra,rb`.
///
/// [PPC-Book1 p:52 s:3.3.8] `add` places the sum of RA and RB into RT.
fn add(rt: u8, ra: u8, rb: u8) -> u32 {
    word(PpuInstruction::Add {
        rt,
        ra,
        rb,
        oe: false,
        rc: false,
    })
}

/// The loop that marks r3 lost, then `tail`.
fn after_loop(tail: &[u32]) -> Vec<u32> {
    let mut words = twenty_pass_loop();
    words.extend_from_slice(tail);
    words
}

/// Each pair of words that moves r3 into CR or an SPR and then moves that
/// register into r4.
///
/// [PPC-Book1 p:83 s:3.3.13] `mtcrf` writes the CR fields that FXM selects from RS, and `mfcr` places CR into RT.
/// [PPC-Book1 p:124 s:5.1.1] `mtocrf` and `mfocrf` move one CR field between CR and a GPR.
/// [PPC-Book1 p:81 s:3.3.13] `mtspr` copies RS into the SPR that its SPR field names.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT.
/// [PPC-Book3 p:16 s:3.3.3] Problem-state software may read SPRG3 through a second SPR number.
fn round_trips() -> Vec<[u32; 2]> {
    use PpuInstruction as I;
    vec![
        [mtspr(SPR_SPRG0, 3), mfspr(4, SPR_SPRG0)],
        [mtspr(SPR_SPRG0 + 3, 3), word(I::Mfsprg3 { rt: 4 })],
        [word(I::Mtcrf { rs: 3, crm: 0xFF }), word(I::Mfcr { rt: 4 })],
        [
            word(I::Mtocrf { rs: 3, crm: 0x80 }),
            word(I::Mfocrf { rt: 4, crm: 0x80 }),
        ],
        [word(I::Mtvrsave { rs: 3 }), word(I::Mfvrsave { rt: 4 })],
        [word(I::Mtxer { rs: 3 }), word(I::Mfxer { rt: 4 })],
        [mtspr(SPR_SRR0, 3), mfspr(4, SPR_SRR0)],
    ]
}

/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_lost_gpr_moved_through_cr_or_an_spr_leaves_every_site_unresolved() {
    let pairs = round_trips();
    assert_eq!(pairs.len(), 7);
    for pair in pairs {
        let words = after_loop(&[pair[0], pair[1], mtctr(4), bctr()]);
        assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED, "{pair:08x?}");
    }
}

/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_of_a_lost_gpr_moved_through_sprg0_leaves_every_site_unresolved() {
    let words = after_loop(&[mtspr(SPR_SPRG0, 3), mfspr(4, SPR_SPRG0), mtlr(4), PPC_BLR]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// The CTR flow follows the copy that the `mfctr` gives only into a later
/// `mtctr`, so the `blr` takes the lost value only through the mark.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtctr` is `mtspr` of SPR 9, which copies RS into CTR.
/// [PPC-Book1 p:82 s:3.3.13] `mfctr` is `mfspr` of SPR 9, which places CTR into RT.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_of_a_lost_gpr_moved_through_ctr_leaves_every_site_unresolved() {
    let words = after_loop(&[mtctr(3), mfctr(4), mtlr(4), PPC_BLR]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// `mfmsr rt`.
///
/// [PPC-Book3 p:23 s:3.4] `mfmsr` places the MSR into RT, and its word is primary opcode 31 with extended opcode 83.
fn mfmsr(rt: u8) -> u32 {
    (31 << 26) | (u32::from(rt) << 21) | (83 << 1)
}

/// The SLB word with the extended opcode `xo`, RS or RT `rt` and RB `rb`.
///
/// [PPC-Book3 p:51 s:4.11.3.1] `slbmte` is primary opcode 31 with extended opcode 402, RS in bits 6:10 and RB in bits 16:20.
/// [PPC-Book3 p:52 s:4.11] `slbmfev` and `slbmfee` are primary opcode 31 with extended opcodes 851 and 915, RT in bits 6:10 and RB in bits 16:20.
fn slb_word(xo: u32, rt: u8, rb: u8) -> u32 {
    (31 << 26) | (u32::from(rt) << 21) | (u32::from(rb) << 11) | (xo << 1)
}

/// Each pair moves the lost r3 into the MSR or into an SLB entry, from RS,
/// from RB or from both, and then moves that register into r4. The
/// `mtmsrd` has L = 1, so it puts control nowhere.
///
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 1 places bits 48 and 62 of RS into the MSR.
/// [PPC-Book3 p:23 s:3.4] `mfmsr` places the MSR into RT.
/// [PPC-Book3 p:51 s:4.11.3.1] `slbmte` takes the ESID from RB and the VSID and flags from RS.
/// [PPC-Book3 p:52 s:4.11] `slbmfev` and `slbmfee` place SLB entry fields into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_lost_gpr_moved_through_the_msr_or_an_slb_entry_leaves_every_site_unresolved() {
    let pairs = [
        [mtmsrd(3, true), mfmsr(4)],
        [slb_word(402, 3, 5), slb_word(851, 4, 5)],
        [slb_word(402, 5, 3), slb_word(915, 4, 5)],
        [slb_word(402, 3, 3), slb_word(915, 4, 5)],
    ];
    for pair in pairs {
        let words = after_loop(&[pair[0], pair[1], mtctr(4), bctr()]);
        assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED, "{pair:08x?}");
    }
}

/// The lost r3 goes into SPRG0, and the `mfspr` reads SRR0. SRR0 holds
/// no value that the loop marks, so r4 takes no mark from SPRG0.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtspr` copies RS into the SPR that its SPR field names.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_srr0_read_after_a_lost_gpr_moved_into_sprg0_keeps_the_sites_resolved() {
    let words = after_loop(&[mtspr(SPR_SPRG0, 3), mfspr(4, SPR_SRR0), mtctr(4), bctr()]);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// The `blt` makes the `mfspr` a join word, so the move into SPRG0 reaches
/// it only through what the join brings.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_lost_gpr_moved_through_sprg0_across_a_join_leaves_every_site_unresolved() {
    let words = after_loop(&[
        mtspr(SPR_SPRG0, 3),
        skip(0),
        mfspr(4, SPR_SPRG0),
        mtctr(4),
        bctr(),
    ]);
    assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED);
}

/// r5 holds the constant that `addi` builds, which the image names. The
/// lost r3 never goes into SPRG0.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to 0 when RA is 0.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_built_constant_moved_through_sprg0_after_the_loop_keeps_the_sites_resolved() {
    let words = after_loop(&[
        addi(5, 0, NOWHERE.cast_signed()),
        mtspr(SPR_SPRG0, 5),
        mfspr(4, SPR_SPRG0),
        mtctr(4),
        bctr(),
    ]);
    assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED);
}

/// Each word that the scan follows no constant through, with r3 as a
/// source, and the GPR that it computes from r3.
///
/// [PPC-Book1 p:52 s:3.3.8] `add` places the sum of RA and RB into RT.
/// [PPC-Book1 p:51 s:3.3.8] An arithmetic instruction places its result into RT.
/// [PPC-Book1 p:65 s:3.3.11] A logical instruction places its result into RA.
/// [PPC-Book1 p:71 s:3.3.12] A rotate or shift instruction returns its result to a GPR, RA.
/// [PPC-Book1 p:33 s:3.3.2] A load with update places the EA into RA.
/// [PPC-Book1 p:40 s:3.3.3] A store with update places the EA into RA.
fn computing_words() -> Vec<(PpuInstruction, u8)> {
    use PpuInstruction as I;
    vec![
        (
            I::Rlwinm {
                ra: 4,
                rs: 3,
                sh: 0,
                mb: 0,
                me: 31,
                rc: false,
            },
            4,
        ),
        (
            I::Rlwimi {
                ra: 4,
                rs: 3,
                sh: 0,
                mb: 0,
                me: 31,
                rc: false,
            },
            4,
        ),
        (
            I::Add {
                rt: 4,
                ra: 5,
                rb: 3,
                oe: false,
                rc: false,
            },
            4,
        ),
        (
            I::Subf {
                rt: 4,
                ra: 3,
                rb: 5,
                oe: false,
                rc: false,
            },
            4,
        ),
        (
            I::Divd {
                rt: 4,
                ra: 3,
                rb: 5,
                oe: false,
                rc: false,
            },
            4,
        ),
        (
            I::Xor {
                ra: 4,
                rs: 3,
                rb: 5,
                rc: false,
            },
            4,
        ),
        (
            I::Extsw {
                ra: 4,
                rs: 3,
                rc: false,
            },
            4,
        ),
        (
            I::Neg {
                rt: 4,
                ra: 3,
                oe: false,
                rc: false,
            },
            4,
        ),
        (
            I::Mulli {
                rt: 4,
                ra: 3,
                imm: 1,
            },
            4,
        ),
        (
            I::Lwzu {
                rt: 9,
                ra: 3,
                imm: 4,
            },
            3,
        ),
        (
            I::Stdu {
                rs: 9,
                ra: 3,
                imm: 8,
            },
            3,
        ),
        (
            I::Ldux {
                rt: 9,
                ra: 3,
                rb: 5,
            },
            3,
        ),
    ]
}

/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_each_unfollowed_word_computed_from_a_lost_gpr_leaves_every_site_unresolved() {
    let computing = computing_words();
    assert_eq!(computing.len(), 12);
    for (computed, result) in computing {
        let words = after_loop(&[word(computed), mtctr(result), bctr()]);
        assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED, "{computed:?}");
    }
}

/// Each word computes its result from GPRs that hold no lost mark, or
/// loads it from storage.
///
/// [PPC-Book1 p:52 s:3.3.8] `add` places the sum of RA and RB into RT.
/// [PPC-Book1 p:33 s:3.3.2] A load with update loads RT from storage and places the EA into RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_unfollowed_word_with_no_lost_source_keeps_the_sites_resolved() {
    let lwzu = word(PpuInstruction::Lwzu {
        rt: 4,
        ra: 3,
        imm: 4,
    });
    let cases = [
        [add(4, 5, 6), mtctr(4)],
        [add(3, 5, 6), mtctr(3)],
        [lwzu, mtctr(4)],
    ];
    for case in cases {
        let words = after_loop(&[case[0], case[1], bctr()]);
        assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED, "{case:08x?}");
    }
}

/// The `mtctr` at word 5 takes the r4 that the `rlwinm` computes, and its
/// mark names the loop join at word 2.
///
/// [PPC-Book1 p:71 s:3.3.12] A rotate or shift instruction returns its result to a GPR, RA.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_unfollowed_word_computed_from_a_loop_gpr_names_the_loop_join() {
    let rlwinm = word(PpuInstruction::Rlwinm {
        ra: 4,
        rs: 3,
        sh: 0,
        mb: 0,
        me: 31,
        rc: false,
    });
    let words = after_loop(&[rlwinm, mtctr(4), bctr()]);
    let segments = [code(VECTOR, &words), window(&[]), quiet()];
    let losses = hypercall_losses(&elf(0, &segments)).expect("the ELF scans");
    let at = |index: u64| VECTOR + 4 * index;
    let expected = HypercallLosses {
        lost_targets: vec![at(6)],
        dropped: vec![DroppedValue {
            address: at(5),
            drops: vec![JoinDrop {
                join: at(2),
                register: JoinRegister::Gpr(3),
                brought: None,
            }],
            more: false,
        }],
    };
    assert_eq!(losses, expected);
}

/// The ordinals with `words` at the vector, the window, the second window
/// and 65 descriptors of the vector's word. Their TOCs are 0x5000 up by
/// 0x10, which name no word of the window.
fn tocs_sites(words: &[u32]) -> Vec<(u64, Option<u16>)> {
    let descriptors: Vec<u8> = (0..=RAISED_LIMIT)
        .flat_map(|toc| descriptor(VECTOR, u64::from(NOWHERE + 0x10 * toc)))
        .collect();
    let segments = [
        code(VECTOR, words),
        window(&[]),
        quiet(),
        data_bytes(DESCRIPTORS, descriptors),
    ];
    ordinals(0, &segments)
}

/// The join at the `or` or the `add` marks r2 with drops of TOCs alone.
/// The `or` copies r2, so r5 holds a TOC. The `add` gives r5 a value that
/// no descriptor need give.
///
/// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
/// [PPC-Book1 p:52 s:3.3.8] `add` places the sum of RA and RB into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_add_to_an_r2_of_tocs_leaves_every_site_unresolved() {
    let copy = word(PpuInstruction::Or {
        ra: 5,
        rs: 2,
        rb: 2,
        rc: false,
    });
    assert_eq!(
        tocs_sites(&[skip(0), copy, mtctr(5), bctr()]),
        WINDOW_RESOLVED
    );
    assert_eq!(
        tocs_sites(&[skip(0), add(5, 2, 6), mtctr(5), bctr()]),
        EVERY_UNRESOLVED
    );
}

/// The ordinals with `words` at [`AWAY`], the window and the second
/// window. A `b` at [`VECTOR`] goes to [`AWAY`].
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
fn away_sites(words: &[u32]) -> Vec<(u64, Option<u16>)> {
    let jump = word(PpuInstruction::B {
        offset: 0x6F00,
        aa: false,
        link: false,
    });
    let segments = [
        code(VECTOR, &[jump]),
        code(AWAY, words),
        window(&[]),
        quiet(),
    ];
    ordinals(0, &segments)
}

/// The loop adds 0x10 to r3 from 0x5000, so the class of r3 names no word
/// after a `li r11`. The `add` of 8 gives r5 the values 8 past a multiple
/// of 0x10, the window's `sc` among them.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0.
/// [PPC-Book1 p:52 s:3.3.8] `add` places the sum of RA and RB into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_an_add_to_a_loop_gpr_of_one_class_leaves_every_site_unresolved() {
    let lp = [
        addi(3, 0, NOWHERE.cast_signed()),
        addi(3, 3, 0x10),
        bc(0b01100, -4),
    ];
    let with = |tail: &[u32]| {
        let mut words = lp.to_vec();
        words.extend_from_slice(tail);
        away_sites(&words)
    };
    assert_eq!(with(&[mtctr(3), bctr()]), WINDOW_RESOLVED);
    assert_eq!(
        with(&[addi(4, 0, 8), add(5, 3, 4), mtctr(5), bctr()]),
        EVERY_UNRESOLVED
    );
}

/// The routine moves a copy of LR through VRSAVE, SPRG0 or CR and returns
/// to the value that it reads back.
///
/// [PPC-Book1 p:82 s:3.3.13] `mflr` copies LR into RT.
/// [PPC-Book1 p:81 s:3.3.13] `mtspr` copies RS into the SPR that its SPR field names.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT.
/// [PPC-Book1 p:83 s:3.3.13] `mtcrf` writes the CR fields that FXM selects from RS, and `mfcr` places CR into RT.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_return_to_a_copy_of_lr_moved_through_an_spr_or_cr_leaves_every_site_unresolved() {
    use PpuInstruction as I;
    let moves = [
        [word(I::Mtvrsave { rs: 0 }), word(I::Mfvrsave { rt: 12 })],
        [mtspr(SPR_SPRG0, 0), mfspr(12, SPR_SPRG0)],
        [
            word(I::Mtcrf { rs: 0, crm: 0xFF }),
            word(I::Mfcr { rt: 12 }),
        ],
    ];
    for [into, out_of] in moves {
        let routine = [mflr(0), into, out_of, mtlr(12), PPC_BLR];
        assert_eq!(
            ordinals_with_routine(&routine),
            [(SC, None)],
            "{into:08x} {out_of:08x}"
        );
    }
}
