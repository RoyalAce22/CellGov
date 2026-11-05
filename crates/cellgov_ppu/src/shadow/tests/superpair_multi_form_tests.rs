//! The pairs whose halves take more than one form: `rldicr; oris`,
//! `or; or`, `or; bc`, `or; lwz`, `or; bctrl`, `ld; addi` and `lwz; or`.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::encode::encode;
use crate::instruction::fuzz::PpuOutcomeClass;
use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::build_from_words;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::EffectKind;

fn word(insn: I) -> u32 {
    encode(&insn).expect("encodes")
}

/// What the shadow holds at 0 for `a` then `b`.
fn fused(a: I, b: I) -> Option<I> {
    build_from_words(0, &[word(a), word(b)]).get(0)
}

fn or(ra: u8, rs: u8, rb: u8, rc: bool) -> I {
    I::Or { ra, rs, rb, rc }
}

/// Runs `pair` on `state`, with `page` mapped at address 0.
fn run(pair: FusedPair, state: &mut PpuState, page: &[u8]) -> ExecuteVerdict {
    let views = [cellgov_mem::RegionView::plain(0, page)];
    execute(
        &I::Fused(pair),
        state,
        cellgov_event::UnitId::new(0),
        &views,
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    )
}

/// Each pair fuses with each half in the forms the shadow gives it:
/// `sldi` for an rldicr, `mr` for an or, a quickened `bc`, and `li` for an
/// addi with RA = 0.
#[test]
fn each_pair_fuses_in_every_form() {
    let rldicr = |sh, me, rc| I::Rldicr {
        ra: 3,
        rs: 4,
        sh,
        me,
        rc,
    };
    let oris = I::Oris {
        ra: 5,
        rs: 3,
        imm: 0x1234,
    };
    let bc = |bo| I::Bc {
        bo,
        bi: 2,
        offset: -16,
        aa: false,
        link: false,
    };
    let ld = I::Ld {
        rt: 0,
        ra: 1,
        imm: 16,
    };
    let lwz = |rt, ra| I::Lwz { rt, ra, imm: 8 };
    let cases = [
        (
            rldicr(8, 40, true),
            oris,
            FusedPair::RldicrOris {
                ra: 3,
                rs: 4,
                sh: 8,
                me: 40,
                rc: true,
                oris_ra: 5,
                oris_rs: 3,
                imm: 0x1234,
            },
        ),
        (
            rldicr(4, 59, false),
            oris,
            FusedPair::RldicrOris {
                ra: 3,
                rs: 4,
                sh: 4,
                me: 59,
                rc: false,
                oris_ra: 5,
                oris_rs: 3,
                imm: 0x1234,
            },
        ),
        (
            or(3, 4, 5, false),
            or(6, 3, 7, true),
            FusedPair::OrOr {
                ra1: 3,
                rs1: 4,
                rb1: 5,
                rc1: false,
                ra2: 6,
                rs2: 3,
                rb2: 7,
                rc2: true,
            },
        ),
        (
            or(3, 4, 4, false),
            or(6, 3, 3, false),
            FusedPair::OrOr {
                ra1: 3,
                rs1: 4,
                rb1: 4,
                rc1: false,
                ra2: 6,
                rs2: 3,
                rb2: 3,
                rc2: false,
            },
        ),
        (
            or(3, 4, 5, true),
            bc(12),
            FusedPair::OrBc {
                ra: 3,
                rs: 4,
                rb: 5,
                rc: true,
                bo: 12,
                bi: 2,
                target_offset: -16,
            },
        ),
        (
            or(3, 4, 4, false),
            bc(16),
            FusedPair::OrBc {
                ra: 3,
                rs: 4,
                rb: 4,
                rc: false,
                bo: 16,
                bi: 2,
                target_offset: -16,
            },
        ),
        (
            or(3, 3, 5, false),
            lwz(6, 3),
            FusedPair::OrLwz {
                ra: 3,
                rs: 3,
                rb: 5,
                rc: false,
                rt: 6,
                lwz_ra: 3,
                offset: 8,
            },
        ),
        (
            or(3, 4, 4, false),
            I::Bcctr {
                bo: 20,
                bi: 0,
                link: true,
            },
            FusedPair::OrBctrl {
                ra: 3,
                rs: 4,
                rb: 4,
                rc: false,
                bo: 20,
                bi: 0,
            },
        ),
        (
            ld,
            I::Addi {
                rt: 3,
                ra: 3,
                imm: -8,
            },
            FusedPair::LdAddi {
                rt: 0,
                offset: 16,
                addi_rt: 3,
                addi_ra: 3,
                imm: -8,
            },
        ),
        (
            ld,
            I::Addi {
                rt: 3,
                ra: 0,
                imm: -8,
            },
            FusedPair::LdAddi {
                rt: 0,
                offset: 16,
                addi_rt: 3,
                addi_ra: 0,
                imm: -8,
            },
        ),
        (
            lwz(3, 4),
            or(5, 3, 3, true),
            FusedPair::LwzOr {
                rt: 3,
                ra: 4,
                offset: 8,
                or_ra: 5,
                or_rs: 3,
                or_rb: 3,
                rc: true,
            },
        ),
    ];
    for (a, b, pair) in cases {
        assert_eq!(fused(a, b), Some(I::Fused(pair)), "{a:?} + {b:?}");
    }
}

/// Each of these pairs stays split:
/// - a bctr, which does not link
/// - a conditional bcctrl
/// - a bc that links
/// - a bc with AA = 1, whose target is an absolute address
/// - an ld from a base other than r1
#[test]
fn the_guards_keep_these_pairs_split() {
    let cases = [
        (
            or(3, 4, 5, false),
            I::Bcctr {
                bo: 20,
                bi: 0,
                link: false,
            },
        ),
        (
            or(3, 4, 5, false),
            I::Bcctr {
                bo: 12,
                bi: 2,
                link: true,
            },
        ),
        (
            or(3, 4, 5, false),
            I::Bc {
                bo: 12,
                bi: 2,
                offset: 8,
                aa: false,
                link: true,
            },
        ),
        // [PPC-Book1 p:24 s:2.4] bca: AA = 1 makes the target EXTS(BD||0b00).
        (
            or(3, 4, 5, false),
            I::Bc {
                bo: 12,
                bi: 2,
                offset: 8,
                aa: true,
                link: false,
            },
        ),
        (
            I::Ld {
                rt: 0,
                ra: 2,
                imm: 16,
            },
            I::Addi {
                rt: 3,
                ra: 3,
                imm: 1,
            },
        ),
    ];
    for (a, b) in cases {
        assert!(
            !matches!(fused(a, b), Some(I::Fused(_))),
            "{a:?} + {b:?} fused"
        );
    }
}

/// [PPC-Book1 p:72 s:3.3.12] rldicr: ROTL64 of RS, ANDed with MASK(0, ME).
/// [PPC-Book1 p:66 s:3.3.11] oris ORs UI into bits 32:47, read after the rldicr.
#[test]
fn the_oris_reads_what_the_rldicr_wrote() {
    let mut state = PpuState::new();
    state.set_gpr(4, 0x1234_5678_9ABC_DEF0);
    let verdict = run(
        FusedPair::RldicrOris {
            ra: 3,
            rs: 4,
            sh: 8,
            me: 55,
            rc: true,
            oris_ra: 5,
            oris_rs: 3,
            imm: 0x0101,
        },
        &mut state,
        &[],
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!(state.gpr[3], 0x3456_789A_BCDE_F000);
    assert_eq!(state.gpr[5], 0x3456_789A_BDDF_F000);
    assert_eq!(
        [state.cr_bit(0), state.cr_bit(1), state.cr_bit(2)],
        [false, true, false]
    );
}

/// [PPC-Book1 p:67 s:3.3.11] the second or reads the first or's RA, and
/// only the recording one sets CR0.
#[test]
fn the_second_or_reads_what_the_first_wrote() {
    let mut state = PpuState::new();
    state.set_gpr(4, 0xF0);
    state.set_gpr(5, 0x0F);
    state.set_gpr(7, 0x8000_0000_0000_0000);
    let verdict = run(
        FusedPair::OrOr {
            ra1: 3,
            rs1: 4,
            rb1: 5,
            rc1: false,
            ra2: 6,
            rs2: 3,
            rb2: 7,
            rc2: true,
        },
        &mut state,
        &[],
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!(state.gpr[3], 0xFF);
    assert_eq!(state.gpr[6], 0x8000_0000_0000_00FF);
    assert_eq!(
        [state.cr_bit(0), state.cr_bit(1), state.cr_bit(2)],
        [true, false, false]
    );
}

/// [PPC-Book1 p:24 s:2.4] the bc reads the CR0 the recording or set, and
/// branches from its own address.
#[test]
fn the_bc_reads_the_cr0_the_or_set() {
    for (r4, verdict, pc) in [
        (0u64, ExecuteVerdict::Branch, 0x1044u64),
        (1, ExecuteVerdict::Continue, 0x1000),
    ] {
        let mut state = PpuState::new();
        state.pc = 0x1000;
        state.set_gpr(4, r4);
        let got = run(
            FusedPair::OrBc {
                ra: 3,
                rs: 4,
                rb: 5,
                rc: true,
                bo: 12,
                bi: 2,
                target_offset: 0x40,
            },
            &mut state,
            &[],
        );
        assert_eq!((got, state.pc), (verdict, pc), "r4 {r4:#x}");
    }
}

/// [PPC-Book1 p:37 s:3.3.2] the lwz EA reads the or's result when its base
/// is the or's RA.
#[test]
fn the_lwz_takes_its_base_from_the_or() {
    let mut page = [0u8; 0x40];
    page[0x18..0x1C].copy_from_slice(&0x8765_4321u32.to_be_bytes());
    let mut state = PpuState::new();
    state.set_gpr(3, 0x10);
    state.set_gpr(5, 0x08);
    let verdict = run(
        FusedPair::OrLwz {
            ra: 3,
            rs: 3,
            rb: 5,
            rc: false,
            rt: 6,
            lwz_ra: 3,
            offset: 0,
        },
        &mut state,
        &page,
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!((state.gpr[3], state.gpr[6]), (0x18, 0x8765_4321));
}

/// A refused lwz leaves the or's RA and CR0 as they were, so the split
/// rerun of `or rA,rA,rB` reads its old RA.
#[test]
fn a_refused_or_lwz_writes_no_register() {
    let mut state = PpuState::new();
    state.set_gpr(3, 1);
    state.set_gpr(5, 2);
    let before = state.fingerprint();
    let verdict = run(
        FusedPair::OrLwz {
            ra: 3,
            rs: 3,
            rb: 5,
            rc: true,
            rt: 6,
            lwz_ra: 7,
            offset: 0x100,
        },
        &mut state,
        &[0; 0x40],
    );
    assert!(
        matches!(verdict, ExecuteVerdict::MemFault(_)),
        "{verdict:?}"
    );
    assert_eq!(state.fingerprint(), before);
}

/// [PPC-Book1 p:25 s:2.4] bcctrl sets LR to its own address + 4 and
/// branches to CTR with the low two bits cleared.
#[test]
fn the_bctrl_links_past_the_pair_and_branches_to_ctr() {
    let mut state = PpuState::new();
    state.pc = 0x1000;
    state.set_ctr(0x2003);
    state.set_gpr(4, 7);
    let verdict = run(
        FusedPair::OrBctrl {
            ra: 3,
            rs: 4,
            rb: 4,
            rc: false,
            bo: 20,
            bi: 0,
        },
        &mut state,
        &[],
    );
    assert_eq!(verdict, ExecuteVerdict::Branch);
    assert_eq!((state.gpr[3], state.lr(), state.pc), (7, 0x1008, 0x2000));
}

/// [PPC-Book1 p:51 s:3.3.8] the addi reads its RA after the ld wrote RT,
/// and an RA of 0 is a literal 0 even when the ld wrote r0.
#[test]
fn the_addi_reads_what_the_ld_wrote() {
    const LOADED: u64 = 0x1111_2222_3333_4444;
    let mut page = [0u8; 0x40];
    page[0x20..0x28].copy_from_slice(&LOADED.to_be_bytes());
    for (rt, addi_ra, sum) in [(3u8, 3u8, LOADED - 4), (0, 0, (-4i64) as u64)] {
        let mut state = PpuState::new();
        state.set_gpr(1, 0x10);
        let verdict = run(
            FusedPair::LdAddi {
                rt,
                offset: 0x10,
                addi_rt: 4,
                addi_ra,
                imm: -4,
            },
            &mut state,
            &page,
        );
        assert_eq!(verdict, ExecuteVerdict::Continue);
        assert_eq!(
            (state.gpr[rt as usize], state.gpr[4]),
            (LOADED, sum),
            "addi_ra {addi_ra}"
        );
    }
}

/// [PPC-Book1 p:67 s:3.3.11] the or reads its RS after the lwz wrote RT.
#[test]
fn the_or_reads_what_the_lwz_wrote() {
    let mut page = [0u8; 0x40];
    page[0x08..0x0C].copy_from_slice(&0x0000_F000u32.to_be_bytes());
    let mut state = PpuState::new();
    state.set_gpr(6, 0x0F);
    let verdict = run(
        FusedPair::LwzOr {
            rt: 3,
            ra: 0,
            offset: 8,
            or_ra: 5,
            or_rs: 3,
            or_rb: 6,
            rc: false,
        },
        &mut state,
        &page,
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!((state.gpr[3], state.gpr[5]), (0xF000, 0xF00F));
}

#[test]
fn each_pair_declares_the_effects_and_outcomes_of_its_halves() {
    use PpuOutcomeClass::{Branch, Continue, MemoryFault};
    let load = (
        vec![EffectKind::SharedReadIntent],
        vec![Continue, MemoryFault],
    );
    let cases = [
        (
            FusedPair::RldicrOris {
                ra: 3,
                rs: 4,
                sh: 1,
                me: 62,
                rc: false,
                oris_ra: 5,
                oris_rs: 3,
                imm: 1,
            },
            (vec![], vec![Continue]),
        ),
        (
            FusedPair::OrOr {
                ra1: 3,
                rs1: 4,
                rb1: 5,
                rc1: false,
                ra2: 6,
                rs2: 3,
                rb2: 7,
                rc2: false,
            },
            (vec![], vec![Continue]),
        ),
        (
            FusedPair::OrBc {
                ra: 3,
                rs: 4,
                rb: 5,
                rc: true,
                bo: 12,
                bi: 2,
                target_offset: 8,
            },
            (vec![], vec![Continue, Branch]),
        ),
        (
            FusedPair::OrBctrl {
                ra: 3,
                rs: 4,
                rb: 5,
                rc: false,
                bo: 20,
                bi: 0,
            },
            (vec![], vec![Branch]),
        ),
        (
            FusedPair::OrLwz {
                ra: 3,
                rs: 4,
                rb: 5,
                rc: false,
                rt: 6,
                lwz_ra: 7,
                offset: 0,
            },
            load.clone(),
        ),
        (
            FusedPair::LdAddi {
                rt: 3,
                offset: 0,
                addi_rt: 4,
                addi_ra: 3,
                imm: 1,
            },
            load.clone(),
        ),
        (
            FusedPair::LwzOr {
                rt: 3,
                ra: 4,
                offset: 0,
                or_ra: 5,
                or_rs: 3,
                or_rb: 6,
                rc: false,
            },
            load,
        ),
    ];
    for (pair, (effects, outcomes)) in cases {
        let descriptor = I::Fused(pair).fuzz_descriptor(0);
        assert_eq!(descriptor.effects, &effects[..], "{pair:?}");
        assert_eq!(descriptor.outcomes, &outcomes[..], "{pair:?}");
    }
}

#[test]
fn each_pair_disassembles_as_its_halves() {
    let cases = [
        (
            FusedPair::RldicrOris {
                ra: 3,
                rs: 4,
                sh: 4,
                me: 59,
                rc: false,
                oris_ra: 5,
                oris_rs: 3,
                imm: 0x10,
            },
            "sldi r3, r4, 4; oris r5, r3, 0x10",
        ),
        (
            FusedPair::OrOr {
                ra1: 3,
                rs1: 4,
                rb1: 4,
                rc1: false,
                ra2: 6,
                rs2: 3,
                rb2: 7,
                rc2: true,
            },
            "mr r3, r4; or. r6, r3, r7",
        ),
        // [PPC-Book1 p:153 s:B.2.3] bc 12,2 is beq on CR0. The bc half
        // sits at 4, so its target is 4 + 0x20.
        (
            FusedPair::OrBc {
                ra: 3,
                rs: 4,
                rb: 5,
                rc: true,
                bo: 12,
                bi: 2,
                target_offset: 0x20,
            },
            "or. r3, r4, r5; beq 0x24",
        ),
        (
            FusedPair::OrLwz {
                ra: 3,
                rs: 4,
                rb: 5,
                rc: false,
                rt: 6,
                lwz_ra: 3,
                offset: 8,
            },
            "or r3, r4, r5; lwz r6, 8(r3)",
        ),
        (
            FusedPair::OrBctrl {
                ra: 12,
                rs: 4,
                rb: 4,
                rc: false,
                bo: 20,
                bi: 0,
            },
            "mr r12, r4; bctrl",
        ),
        (
            FusedPair::LdAddi {
                rt: 0,
                offset: 16,
                addi_rt: 1,
                addi_ra: 1,
                imm: 112,
            },
            "ld r0, 16(r1); addi r1, r1, 112",
        ),
        // [PPC-Book1 p:162 s:B.9] li rX,v is addi rX,0,v.
        (
            FusedPair::LdAddi {
                rt: 0,
                offset: 16,
                addi_rt: 3,
                addi_ra: 0,
                imm: -8,
            },
            "ld r0, 16(r1); li r3, -8",
        ),
        (
            FusedPair::LwzOr {
                rt: 3,
                ra: 4,
                offset: 8,
                or_ra: 5,
                or_rs: 3,
                or_rb: 3,
                rc: false,
            },
            "lwz r3, 8(r4); mr r5, r3",
        ),
    ];
    for (pair, text) in cases {
        let rendered = AsmText {
            insn: &I::Fused(pair),
            addr: 0,
            symbols: None,
        }
        .to_string();
        assert_eq!(
            rendered.split_whitespace().collect::<Vec<_>>().join(" "),
            text
        );
    }
}
