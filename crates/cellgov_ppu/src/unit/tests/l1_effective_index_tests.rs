//! Which L1 set the timing model gives a translating thread's line: the
//! set of its effective line, with its real line as the tag.
//!
//! [CBE-Handbook p:137 s:6.1.3.9] The L1 data cache is indexed with effective addresses.
//!
//! [CBE-Handbook p:135 s:6.1] The L1 data cache has real-addressed tags.
//!
//! Every page here is 4 KB. A line at offset 0 of a page whose effective
//! address bit 12 is 0 picks L1 set 0. With bit 12 at 1, it picks set 32.

use crate::instruction::PpuInstruction as I;
use crate::mmu::{Access, Translation};
use crate::state::PpuState;
use crate::timing::{classify, PpeTiming};
use crate::PpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::ppu_issue as k;

const PAGE: u64 = 0x1000;

/// Four pages whose effective and real lines all pick L1 set 0.
const SET_0_EA: [u64; 4] = [0xC010_0000, 0xC010_2000, 0xC010_4000, 0xC010_6000];
const SET_0_RA: [u64; 4] = [0x10_0000, 0x10_2000, 0x10_4000, 0x10_6000];
/// A page whose real line picks set 0 and whose effective line picks
/// set 32.
const REAL_SET_0_EA: u64 = 0xC010_9000;
const REAL_SET_0_RA: u64 = 0x10_8000;
/// A page whose effective and real lines both pick set 0.
const BOTH_SET_0_EA: u64 = 0xC010_A000;
const BOTH_SET_0_RA: u64 = 0x10_A000;
/// A page whose effective line picks set 0 and whose real line picks
/// set 32.
const EA_SET_0_EA: u64 = 0xC010_C000;
const EA_SET_0_RA: u64 = 0x10_D000;
/// A page whose effective and real lines both pick set 32.
const BOTH_SET_32_EA: u64 = 0xC010_F000;
const BOTH_SET_32_RA: u64 = 0x10_F000;

/// One real page, [`ALIAS_RA`], through three effective pages: two that
/// pick set 0 and one that picks set 32.
const ALIAS_RA: u64 = 0x20_0000;
const ALIAS_LOW_EA: u64 = 0xC020_0000;
const ALIAS_TWIN_EA: u64 = 0xC020_4000;
const ALIAS_HIGH_EA: u64 = 0xC020_3000;
/// A page no other test page shares a line with.
const COLD_EA: u64 = 0xC020_8000;
const COLD_RA: u64 = 0x20_8000;

/// A page whose effective line picks set 32 and whose real line picks
/// set 0.
const SKEW_EA: u64 = 0xC030_1000;
const SKEW_RA: u64 = 0x30_0000;
/// A page whose effective and real lines both pick set 32.
const EVEN_EA: u64 = 0xC030_3000;
const EVEN_RA: u64 = 0x34_1000;
/// Real lines this far apart share an L2 set.
const L2_SET_STRIDE: u64 = k::PPU_L2_SETS as u64 * k::PPU_CACHE_LINE_BYTES;
/// Pages whose real lines share [`SKEW_RA`]'s L2 set: twice the L2's
/// ways. Their effective lines pick L1 set 0.
const EVICTORS: u64 = 2 * k::PPU_L2_WAYS as u64;
const EVICTOR_EA: u64 = 0xC040_0000;

/// The page where a crossing access starts, and the page after it.
const PAGE_EA: u64 = 0xC000_4000;
const PAGE_RA: u64 = 0x6000;
const NEXT_EA: u64 = PAGE_EA + PAGE;
/// The real page after [`PAGE_RA`]. Its lines pick set 32, as
/// [`NEXT_EA`]'s do.
const ADJACENT_RA: u64 = PAGE_RA + PAGE;
/// A real page whose lines pick set 0.
const FAR_RA: u64 = 0xA000;
/// Two more pages over [`FAR_RA`]: one picks set 32, one picks set 0.
const FAR_HIGH_EA: u64 = 0xC000_9000;
const FAR_LOW_EA: u64 = 0xC000_8000;
const LAST_LINE_EA: u64 = NEXT_EA - 0x80;

/// A page whose effective address bit 12 is 0, over a real page whose
/// bit 12 is 1. [`CONTIG_NEXT_EA`] maps to the real page after it, so an
/// access that crosses from one to the other has no tail.
const CONTIG_EA: u64 = 0xC050_0000;
const CONTIG_RA: u64 = 0x50_1000;
/// The page after [`CONTIG_EA`]. Its effective lines pick set 32, and its
/// real lines, on [`CONTIG_NEXT_RA`], pick set 0.
const CONTIG_NEXT_EA: u64 = CONTIG_EA + PAGE;
const CONTIG_NEXT_RA: u64 = CONTIG_RA + PAGE;
/// A second page over [`CONTIG_NEXT_RA`], whose effective lines pick
/// set 0.
const CONTIG_ALIAS_EA: u64 = 0xC050_4000;
const CONTIG_LAST_LINE_EA: u64 = CONTIG_NEXT_EA - 0x80;

/// The `n`th page of the [`EVICTORS`], from 1.
const fn evictor(n: u64) -> (u64, u64) {
    (EVICTOR_EA + n * 2 * PAGE, SKEW_RA + n * L2_SET_STRIDE)
}

/// A unit that holds a data translation of every page above, with
/// [`NEXT_EA`] at `next_ra`.
fn unit(next_ra: u64) -> PpuExecutionUnit {
    let mut pages = vec![
        (BOTH_SET_0_EA, BOTH_SET_0_RA),
        (REAL_SET_0_EA, REAL_SET_0_RA),
        (EA_SET_0_EA, EA_SET_0_RA),
        (BOTH_SET_32_EA, BOTH_SET_32_RA),
        (ALIAS_LOW_EA, ALIAS_RA),
        (ALIAS_TWIN_EA, ALIAS_RA),
        (ALIAS_HIGH_EA, ALIAS_RA),
        (COLD_EA, COLD_RA),
        (SKEW_EA, SKEW_RA),
        (EVEN_EA, EVEN_RA),
        (PAGE_EA, PAGE_RA),
        (NEXT_EA, next_ra),
        (FAR_HIGH_EA, FAR_RA),
        (FAR_LOW_EA, FAR_RA),
        (CONTIG_EA, CONTIG_RA),
        (CONTIG_NEXT_EA, CONTIG_NEXT_RA),
        (CONTIG_ALIAS_EA, CONTIG_NEXT_RA),
    ];
    pages.extend(SET_0_EA.into_iter().zip(SET_0_RA));
    pages.extend((1..=EVICTORS).map(evictor));
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    for (ea, ra) in pages {
        unit.translations.insert(
            Access::Load,
            Translation {
                ea,
                ra,
                len: PAGE,
                writable: true,
                inhibited: false,
                guarded: false,
                changed: true,
            },
        );
    }
    unit
}

/// `ld r6,0(r3)`, whose result no step uses.
const fn warm() -> I {
    I::Ld {
        rt: 6,
        ra: 3,
        imm: 0,
    }
}

/// `ld r4,0(r3)`.
const fn ld() -> I {
    I::Ld {
        rt: 4,
        ra: 3,
        imm: 0,
    }
}

/// `addi r5,r4,1`.
const fn use_r4() -> I {
    I::Addi {
        rt: 5,
        ra: 4,
        imm: 1,
    }
}

/// `std r7,0(r3)`.
const fn std_() -> I {
    I::Std {
        rs: 7,
        ra: 3,
        imm: 0,
    }
}

const fn dcbt() -> I {
    I::Dcbt {
        ra: 0,
        rb: 3,
        th: 0,
    }
}

const fn dcbf() -> I {
    I::Dcbf { ra: 0, rb: 3, l: 0 }
}

const fn dcbz() -> I {
    I::Dcbz { ra: 0, rb: 3 }
}

/// `addi r9,r9,1`.
const fn add_to_r9() -> I {
    I::Addi {
        rt: 9,
        ra: 9,
        imm: 1,
    }
}

/// The length of an [`add_to_r9`] chain that takes at least twice the
/// cycles of a touch's trip to the L2. A touch before the chain arrives
/// before the chain ends.
const SPACER: usize = 2 * k::PPU_L2_LOAD_USE_CYCLES as usize;

/// The length of an [`add_to_r9`] chain that outlasts the hold a reload
/// from memory puts on its L1 set. A load after the chain waits for no
/// set.
const DRAIN: usize = (k::PPU_MEMORY_LOAD_USE_CYCLES + k::PPU_LOAD_MISS_ENTRY_REUSE_CYCLES) as usize;

/// The cycles from an `ld` of `last` to a use of its result, after
/// `before`.
///
/// Each step runs with r3 at its effective address, on [`unit`] with
/// [`NEXT_EA`] at `next_ra`. The unit moves each step's storage
/// addresses to real before the timing model retires it.
fn reload_gap(next_ra: u64, before: &[(I, u64)], last: u64) -> u64 {
    let unit = unit(next_ra);
    let mut state = PpuState::new();
    let mut timing = PpeTiming::new();
    let steps = before.iter().copied().chain([(ld(), last), (use_r4(), 0)]);
    let mut cycles = Vec::with_capacity(before.len() + 2);
    for (i, (insn, r3)) in steps.enumerate() {
        state.set_gpr(3, r3);
        let at = 0x1000 + 4 * i as u64;
        let (mut ops, n) = classify(&insn, &state);
        unit.real_op_addresses(&mut ops[..n]);
        let issued = timing.retire_instruction(at, &ops[..n], at + 4 * n.max(1) as u64);
        cycles.push(issued[0]);
    }
    assert_eq!(cycles.len(), before.len() + 2);
    cycles[cycles.len() - 1] - cycles[cycles.len() - 2]
}

/// The gap of an `ld` of the first [`SET_0_EA`] page to its use, after
/// loads of the four [`SET_0_EA`] pages and of `fifth`.
fn set_0_reload_gap(fifth: u64) -> u64 {
    let mut before: Vec<(I, u64)> = SET_0_EA.iter().map(|&ea| (warm(), ea)).collect();
    before.push((warm(), fifth));
    reload_gap(ADJACENT_RA, &before, SET_0_EA[0])
}

#[test]
fn a_fifth_line_whose_real_line_shares_an_l1_set_but_whose_effective_bit_12_differs_evicts_nothing()
{
    let elsewhere = set_0_reload_gap(BOTH_SET_32_EA);
    let real_shared = set_0_reload_gap(REAL_SET_0_EA);
    let effective_shared = set_0_reload_gap(BOTH_SET_0_EA);
    assert_eq!(real_shared, elsewhere);
    assert!(
        effective_shared > real_shared,
        "a fifth line in the set evicts the first: {effective_shared} vs {real_shared}"
    );
}

#[test]
fn a_fifth_line_whose_effective_line_shares_an_l1_set_but_whose_real_bit_12_differs_evicts_the_first(
) {
    let elsewhere = set_0_reload_gap(BOTH_SET_32_EA);
    let effective_shared = set_0_reload_gap(EA_SET_0_EA);
    assert_eq!(effective_shared, set_0_reload_gap(BOTH_SET_0_EA));
    assert!(
        effective_shared > elsewhere,
        "the reload misses the L1: {effective_shared} vs {elsewhere}"
    );
}

/// [CBE-Handbook p:138 s:6.1.3.10] A line found at the other congruence class is invalidated, and the data from the L2 is placed at the congruence class of the effective address.
#[test]
fn a_load_through_an_alias_in_the_other_l1_set_reloads_the_line_from_the_l2() {
    let twin = reload_gap(ADJACENT_RA, &[(warm(), ALIAS_LOW_EA)], ALIAS_TWIN_EA);
    let other = reload_gap(ADJACENT_RA, &[(warm(), ALIAS_LOW_EA)], ALIAS_HIGH_EA);
    let cold = reload_gap(ADJACENT_RA, &[(warm(), ALIAS_LOW_EA)], COLD_EA);
    assert!(
        twin < other && other < cold,
        "an L2 hit: {twin} < {other} < {cold}"
    );
}

/// [CBE-Handbook p:138 s:6.1.3.10] A line found at the other congruence class is invalidated, and the data from the L2 is placed at the congruence class of the effective address.
#[test]
fn a_load_through_the_first_alias_after_one_through_the_other_l1_set_reloads_from_the_l2() {
    let stay = [(warm(), ALIAS_LOW_EA), (warm(), ALIAS_TWIN_EA)];
    let moved = [(warm(), ALIAS_LOW_EA), (warm(), ALIAS_HIGH_EA)];
    let stayed = reload_gap(ADJACENT_RA, &stay, ALIAS_LOW_EA);
    let back = reload_gap(ADJACENT_RA, &moved, ALIAS_LOW_EA);
    assert!(
        back > stayed,
        "the line left the first set: {back} vs {stayed}"
    );
}

/// [CBE-Handbook p:138 s:6.1.3.10] A line found at the other congruence class is invalidated.
///
/// [CBE-Handbook p:135 s:6.1.3.1] The L1 data cache does not allocate on a write.
#[test]
fn a_store_through_an_alias_in_the_other_l1_set_drops_the_l1_copy() {
    let twin = [(warm(), ALIAS_LOW_EA), (std_(), ALIAS_TWIN_EA + 0x40)];
    let other = [(warm(), ALIAS_LOW_EA), (std_(), ALIAS_HIGH_EA + 0x40)];
    let kept = reload_gap(ADJACENT_RA, &twin, ALIAS_LOW_EA);
    let dropped = reload_gap(ADJACENT_RA, &other, ALIAS_LOW_EA);
    assert!(
        dropped > kept,
        "the store dropped the copy: {dropped} vs {kept}"
    );
}

/// [CBE-Handbook p:145 s:6.1.6.2] A touch with TH = '0000' acts like a load and reloads the L1.
#[test]
fn a_touch_through_an_alias_in_the_other_l1_set_moves_the_line_there() {
    let after_touch = |touched: u64| {
        let mut steps = vec![(warm(), ALIAS_LOW_EA), (dcbt(), touched)];
        steps.extend(std::iter::repeat_n((add_to_r9(), 0), SPACER));
        reload_gap(ADJACENT_RA, &steps, ALIAS_LOW_EA)
    };
    let kept = after_touch(ALIAS_TWIN_EA);
    let moved = after_touch(ALIAS_HIGH_EA);
    assert!(moved > kept, "the touch moved the line: {moved} vs {kept}");
}

/// [CBE-Handbook p:138 s:6.1.3.10] A line found at the other congruence class is invalidated, and the data from the L2 is placed at the congruence class of the effective address.
#[test]
fn a_second_touch_through_the_other_l1_set_arrives_no_earlier_than_the_first() {
    let twin = [(dcbt(), ALIAS_LOW_EA), (dcbt(), ALIAS_TWIN_EA)];
    let other = [(dcbt(), ALIAS_LOW_EA), (dcbt(), ALIAS_HIGH_EA)];
    let same_set = reload_gap(ADJACENT_RA, &twin, ALIAS_TWIN_EA);
    let other_set = reload_gap(ADJACENT_RA, &other, ALIAS_HIGH_EA);
    assert_eq!(other_set, same_set);
}

/// [CBE-Handbook p:140 s:6.1.5.1] The L2 includes all of the L1 data cache contents.
#[test]
fn an_l2_eviction_drops_the_line_from_the_l1_set_of_its_effective_line() {
    let mut before = vec![(warm(), SKEW_EA)];
    before.extend((1..=EVICTORS).map(|n| (warm(), evictor(n).0)));
    before.extend(std::iter::repeat_n((add_to_r9(), 0), DRAIN));
    let evicted = reload_gap(ADJACENT_RA, &before, SKEW_EA);
    let cold = reload_gap(ADJACENT_RA, &before, COLD_EA);
    assert_eq!(evicted, cold);
}

/// [CBE-Handbook p:147 s:6.1.6.5] `dcbf` invalidates the line in all caches.
#[test]
fn a_dcbf_drops_the_line_from_the_l1_set_of_its_effective_line() {
    let skew = reload_gap(
        ADJACENT_RA,
        &[(warm(), SKEW_EA), (dcbf(), SKEW_EA)],
        SKEW_EA,
    );
    let even = reload_gap(
        ADJACENT_RA,
        &[(warm(), EVEN_EA), (dcbf(), EVEN_EA)],
        EVEN_EA,
    );
    assert_eq!(skew, even);
}

/// [CBE-Handbook p:147 s:6.1.6.4] When the line that `dcbz` addresses is present in the L1, the L1 invalidates it.
#[test]
fn a_dcbz_drops_the_line_from_the_l1_set_of_its_effective_line() {
    let skew = reload_gap(
        ADJACENT_RA,
        &[(warm(), SKEW_EA), (dcbz(), SKEW_EA)],
        SKEW_EA,
    );
    let even = reload_gap(
        ADJACENT_RA,
        &[(warm(), EVEN_EA), (dcbz(), EVEN_EA)],
        EVEN_EA,
    );
    assert_eq!(skew, even);
    let present = reload_gap(ADJACENT_RA, &[(warm(), EVEN_EA)], EVEN_EA);
    assert!(
        even > present,
        "the reload after the dcbz misses the L1: {even} vs {present}"
    );
}

#[test]
fn a_crossing_lds_second_line_takes_the_l1_set_of_its_effective_line() {
    let at = NEXT_EA - 4;
    let warm_with = |second: u64| [(warm(), LAST_LINE_EA), (warm(), second)];
    let adjacent = reload_gap(ADJACENT_RA, &warm_with(NEXT_EA), at);
    let same_set = reload_gap(FAR_RA, &warm_with(FAR_HIGH_EA), at);
    let other_set = reload_gap(FAR_RA, &warm_with(FAR_LOW_EA), at);
    assert_eq!(same_set, adjacent);
    assert!(
        other_set > same_set,
        "the second line misses the L1: {other_set} vs {same_set}"
    );
}

/// [CBE-Handbook p:135 s:6.1] The L1 data cache has an effective-addressed index and real-addressed tags.
///
/// The next page follows in real storage, so the `ld` has no tail. Its
/// second line's effective and real addresses differ in bit 12.
#[test]
fn a_real_contiguous_crossing_lds_second_line_takes_the_l1_set_of_its_effective_line() {
    let at = CONTIG_NEXT_EA - 4;
    assert_eq!(
        unit(ADJACENT_RA).real_operand(at, 8, false),
        (CONTIG_NEXT_RA - 4, None)
    );
    let pair = |first: u64, second: u64| [(warm(), first), (warm(), second)];
    let adjacent = reload_gap(ADJACENT_RA, &pair(LAST_LINE_EA, NEXT_EA), NEXT_EA - 4);
    let crossing = |second: u64| reload_gap(ADJACENT_RA, &pair(CONTIG_LAST_LINE_EA, second), at);
    let effective_set = crossing(CONTIG_NEXT_EA);
    let real_set = crossing(CONTIG_ALIAS_EA);
    assert_eq!(effective_set, adjacent);
    assert!(
        real_set > effective_set,
        "the second line misses the L1: {real_set} vs {effective_set}"
    );
}
