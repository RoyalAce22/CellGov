//! What the timing model charges a translating thread's load or store
//! whose bytes cross into a page that maps to a real page not next to
//! the first page's.
//!
//! [CBE-Handbook p:135 s:6.1] The L1 data cache has real-addressed tags.
//!
//! [CBE-Handbook p:140 s:6.1.5.1] The L2 has a real-address index and real-address tags.

use crate::instruction::PpuInstruction as I;
use crate::mmu::{Access, Translation};
use crate::state::PpuState;
use crate::timing::{classify, Kind, PpeTiming, Tail};
use crate::PpuExecutionUnit;
use cellgov_event::UnitId;

/// The page where each crossing access starts, and its real address.
const PAGE_EA: u64 = 0xC000_4000;
const PAGE_RA: u64 = 0x6000;
/// The page after [`PAGE_EA`].
const NEXT_EA: u64 = PAGE_EA + PAGE;
/// The real page that follows [`PAGE_RA`]. The old pricing read the
/// second line of a crossing access here.
const ADJACENT_RA: u64 = PAGE_RA + PAGE;
/// A real page that does not follow [`PAGE_RA`].
const FAR_RA: u64 = 0x9000;
/// A page that maps to [`ADJACENT_RA`] in every unit.
const ALIAS_EA: u64 = 0xC000_6000;
/// A page that maps to a real page no access of the tests reaches.
const OTHER_EA: u64 = 0xC000_7000;
const OTHER_RA: u64 = 0xB000;
const PAGE: u64 = 0x1000;

/// The last line of [`PAGE_EA`].
const LAST_LINE_EA: u64 = NEXT_EA - 0x80;

/// A unit whose data translations map:
///
/// - [`PAGE_EA`] to [`PAGE_RA`];
/// - [`NEXT_EA`] to `next_ra`;
/// - [`ALIAS_EA`] to [`ADJACENT_RA`];
/// - [`OTHER_EA`] to [`OTHER_RA`].
fn unit(next_ra: u64) -> PpuExecutionUnit {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    for (ea, ra) in [
        (PAGE_EA, PAGE_RA),
        (NEXT_EA, next_ra),
        (ALIAS_EA, ADJACENT_RA),
        (OTHER_EA, OTHER_RA),
    ] {
        unit.translations.insert(
            Access::Load,
            Translation {
                ea,
                ra,
                len: PAGE,
                writable: true,
                inhibited: false,
                changed: true,
            },
        );
    }
    unit
}

/// The cycles from the second-to-last instruction of `steps` to the last.
///
/// Each step runs with r3 at its effective address. The unit moves each
/// step's storage addresses to real before the timing model retires it.
fn last_gap(unit: &PpuExecutionUnit, steps: &[(I, u64)]) -> u64 {
    let mut state = PpuState::new();
    let mut timing = PpeTiming::new();
    let mut cycles = Vec::with_capacity(steps.len());
    for (i, (insn, r3)) in steps.iter().enumerate() {
        state.set_gpr(3, *r3);
        let at = 0x1000 + 4 * i as u64;
        let (mut ops, n) = classify(insn, &state);
        unit.real_op_addresses(&mut ops[..n]);
        let issued = timing.retire_instruction(at, &ops[..n], at + 4 * n.max(1) as u64);
        cycles.push(issued[0]);
    }
    cycles[steps.len() - 1] - cycles[steps.len() - 2]
}

const fn warm() -> I {
    I::Ld {
        rt: 6,
        ra: 3,
        imm: 0,
    }
}

const fn ld() -> I {
    I::Ld {
        rt: 4,
        ra: 3,
        imm: 0,
    }
}

const fn std_() -> I {
    I::Std {
        rs: 4,
        ra: 3,
        imm: 0,
    }
}

/// `lmw r24,0(r3)`: eight words, 32 bytes.
const fn lmw() -> I {
    I::Lmw {
        rt: 24,
        ra: 3,
        imm: 0,
    }
}

/// `stmw r24,0(r3)`: eight words, 32 bytes.
const fn stmw() -> I {
    I::Stmw {
        rs: 24,
        ra: 3,
        imm: 0,
    }
}

/// `addi r5,rN,1`, which uses `rN`.
const fn uses(rn: u8) -> I {
    I::Addi {
        rt: 5,
        ra: rn,
        imm: 1,
    }
}

/// The cycles from the crossing `load` at `at` to a use of `rn`, after
/// loads of [`LAST_LINE_EA`] and `second` fill their lines.
fn load_gap(next_ra: u64, load: I, at: u64, rn: u8, second: u64) -> u64 {
    let steps = [
        (warm(), LAST_LINE_EA),
        (warm(), second),
        (load, at),
        (uses(rn), 0),
    ];
    last_gap(&unit(next_ra), &steps)
}

/// The cycles from a load of `probe` to its use, after the crossing
/// `store` at `at`.
fn probe_gap(next_ra: u64, store: I, at: u64, probe: u64) -> u64 {
    let steps = [(store, at), (ld(), probe), (uses(4), 0)];
    last_gap(&unit(next_ra), &steps)
}

/// `dcbt 0,r3` with TH = 0.
const fn dcbt() -> I {
    I::Dcbt {
        ra: 0,
        rb: 3,
        th: 0,
    }
}

/// `dcbf 0,r3`.
const fn dcbf() -> I {
    I::Dcbf { ra: 0, rb: 3, l: 0 }
}

/// The cycles from the crossing `ld` at the end of [`PAGE_EA`] to its
/// use, after a load of [`LAST_LINE_EA`] and a touch of `touched`.
fn touch_gap(next_ra: u64, touched: u64) -> u64 {
    let steps = [
        (warm(), LAST_LINE_EA),
        (dcbt(), touched),
        (ld(), NEXT_EA - 4),
        (uses(4), 0),
    ];
    last_gap(&unit(next_ra), &steps)
}

/// The cycles from the crossing `ld` at the end of [`PAGE_EA`] to its
/// use, after a flush of `flushed`.
fn flush_gap(next_ra: u64, flushed: u64) -> u64 {
    let steps = [(dcbf(), flushed), (ld(), NEXT_EA - 4), (uses(4), 0)];
    last_gap(&unit(next_ra), &steps)
}

#[test]
fn a_crossing_ld_gets_a_tail_at_the_next_pages_real_address() {
    let mut state = PpuState::new();
    state.set_gpr(3, NEXT_EA - 4);
    let (mut ops, n) = classify(&ld(), &state);
    unit(FAR_RA).real_op_addresses(&mut ops[..n]);
    let Kind::Load { ea, len, tail, .. } = ops[0].kind else {
        panic!("ld classifies as a load: {:?}", ops[0].kind);
    };
    assert_eq!((ea, len), (ADJACENT_RA - 4, 8));
    assert_eq!(tail, Some(Tail { at: 4, ra: FAR_RA }));
}

#[test]
fn a_crossing_ld_whose_next_page_follows_in_real_storage_has_no_tail() {
    let mut state = PpuState::new();
    state.set_gpr(3, NEXT_EA - 4);
    let (mut ops, n) = classify(&ld(), &state);
    unit(ADJACENT_RA).real_op_addresses(&mut ops[..n]);
    assert!(
        matches!(ops[0].kind, Kind::Load { tail: None, .. }),
        "{:?}",
        ops[0].kind
    );
}

#[test]
fn a_crossing_ld_hits_the_l1_line_of_its_second_pieces_real_address() {
    let at = NEXT_EA - 4;
    let warm_far = load_gap(FAR_RA, ld(), at, 4, NEXT_EA);
    let warm_adjacent = load_gap(ADJACENT_RA, ld(), at, 4, NEXT_EA);
    assert_eq!(warm_far, warm_adjacent);
}

#[test]
fn a_crossing_ld_misses_when_only_the_line_after_its_first_piece_is_present() {
    let at = NEXT_EA - 4;
    let cold = load_gap(ADJACENT_RA, ld(), at, 4, OTHER_EA);
    let warm = load_gap(ADJACENT_RA, ld(), at, 4, NEXT_EA);
    assert!(
        cold > warm,
        "a missing second line costs more: {cold} vs {warm}"
    );
    assert_eq!(load_gap(FAR_RA, ld(), at, 4, ALIAS_EA), cold);
}

#[test]
fn a_crossing_lmw_hits_the_l1_line_of_its_second_pieces_real_address() {
    let at = NEXT_EA - 16;
    let warm_far = load_gap(FAR_RA, lmw(), at, 31, NEXT_EA);
    let warm_adjacent = load_gap(ADJACENT_RA, lmw(), at, 31, NEXT_EA);
    assert_eq!(warm_far, warm_adjacent);
}

#[test]
fn a_crossing_lmw_misses_when_only_the_line_after_its_first_piece_is_present() {
    let at = NEXT_EA - 16;
    let cold = load_gap(ADJACENT_RA, lmw(), at, 31, OTHER_EA);
    let warm = load_gap(ADJACENT_RA, lmw(), at, 31, NEXT_EA);
    assert!(
        cold > warm,
        "a missing second line costs more: {cold} vs {warm}"
    );
    assert_eq!(load_gap(FAR_RA, lmw(), at, 31, ALIAS_EA), cold);
}

#[test]
fn a_crossing_std_allocates_the_l2_line_of_its_second_pieces_real_address() {
    let at = NEXT_EA - 4;
    let allocated = probe_gap(ADJACENT_RA, std_(), at, NEXT_EA + 0x40);
    let cold = probe_gap(ADJACENT_RA, std_(), at, OTHER_EA + 0x40);
    assert!(
        cold > allocated,
        "an L2 hit costs less than memory: {allocated} vs {cold}"
    );
    assert_eq!(probe_gap(FAR_RA, std_(), at, NEXT_EA + 0x40), allocated);
    assert_eq!(probe_gap(FAR_RA, std_(), at, ALIAS_EA + 0x40), cold);
}

#[test]
fn a_crossing_stmw_allocates_the_l2_line_of_its_second_pieces_real_address() {
    let at = NEXT_EA - 16;
    let allocated = probe_gap(ADJACENT_RA, stmw(), at, NEXT_EA + 0x40);
    let cold = probe_gap(ADJACENT_RA, stmw(), at, OTHER_EA + 0x40);
    assert!(
        cold > allocated,
        "an L2 hit costs less than memory: {allocated} vs {cold}"
    );
    assert_eq!(probe_gap(FAR_RA, stmw(), at, NEXT_EA + 0x40), allocated);
    assert_eq!(probe_gap(FAR_RA, stmw(), at, ALIAS_EA + 0x40), cold);
}

#[test]
fn a_crossing_ld_waits_for_a_touch_of_its_second_pieces_real_line() {
    let warm = load_gap(ADJACENT_RA, ld(), NEXT_EA - 4, 4, NEXT_EA);
    let touched = touch_gap(ADJACENT_RA, NEXT_EA);
    assert!(
        touched > warm,
        "a line still on its way costs more: {touched} vs {warm}"
    );
    assert_eq!(touch_gap(FAR_RA, NEXT_EA), touched);
}

#[test]
fn a_crossing_ld_waits_for_a_flush_of_its_second_pieces_real_line() {
    let unflushed = flush_gap(ADJACENT_RA, OTHER_EA);
    let flushed = flush_gap(ADJACENT_RA, NEXT_EA);
    assert!(
        flushed > unflushed,
        "a line still busy with a flush costs more: {flushed} vs {unflushed}"
    );
    assert_eq!(flush_gap(FAR_RA, NEXT_EA), flushed);
}
