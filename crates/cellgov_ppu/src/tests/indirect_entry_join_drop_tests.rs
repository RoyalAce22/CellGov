use std::collections::BTreeMap;
use std::iter::once;

use super::constants::Value;
use super::join_flow::{Bounds, EntryWords, JoinFlow, JoinWords};
use super::lost::{join_origins, Origins};
use super::reach::Reach;
use super::tests::word;
use super::toc::FunctionWords;
use super::*;
use crate::caller_census::{JoinDrop, JoinRegister};

/// The address of the first word of the code.
const BASE: u64 = 0x1_0000;

/// `count` paths that each build their own constant in r3 and then take
/// a `blt` to the `mtctr` after the last path. Each path takes three
/// words.
///
/// [PPC-Book1 p:51 s:3.3.8] `addis` adds EXTS(SI || 0x0000) to 0 when RA is 0.
/// [PPC-Book1 p:66 s:3.3.11] `ori` ORs RS with UI and places the result into RA.
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
fn paths(count: u16) -> Vec<u32> {
    let mut words = Vec::new();
    for path in 0..count {
        let skip = i16::try_from(3 * (count - 1 - path) + 1).expect("a short skip");
        words.extend([
            word(PpuInstruction::Addis {
                rt: 3,
                ra: 0,
                imm: 0,
            }),
            word(PpuInstruction::Ori {
                ra: 3,
                rs: 3,
                imm: 0x5000 + 0x10 * path,
            }),
            word(PpuInstruction::Bc {
                bo: 0b01100,
                bi: 0,
                offset: 4 * skip,
                aa: false,
                link: false,
            }),
        ]);
    }
    words.push(word(PpuInstruction::Mtctr { rs: 3 }));
    words
}

/// The drops of one `JoinFlow` run over `words` at [`BASE`] with
/// `bounds`. The run counts the drops when `counting` is set.
fn drops_of(words: Vec<u32>, bounds: &Bounds, counting: bool) -> Vec<(usize, JoinDrop)> {
    let code = Code::new(vec![CodeSegment { vaddr: BASE, words }]);
    let functions = BTreeMap::new();
    let function_words = FunctionWords::new(&code, &functions);
    let every = Value::sorted(Vec::new());
    let join_words = JoinWords::new(&code);
    let reach = Reach::every(&code);
    JoinFlow::run(
        &join_words,
        &function_words.tocs,
        &every,
        bounds,
        &reach,
        &EntryWords::default(),
        counting,
    )
    .drops()
}

/// The counted drops of one `JoinFlow` run.
fn drops(words: Vec<u32>, bounds: &Bounds) -> Vec<(usize, JoinDrop)> {
    drops_of(words, bounds, true)
}

#[test]
fn a_flow_that_does_not_count_gives_the_same_drops_with_no_count() {
    let found = drops_of(paths(17), &Bounds::default(), false);
    assert_eq!(found, vec![r3_drop(17, None)]);
    let counted: Vec<(usize, JoinDrop)> = drops(paths(17), &Bounds::default())
        .into_iter()
        .map(|(slot, drop)| {
            (
                slot,
                JoinDrop {
                    brought: None,
                    ..drop
                },
            )
        })
        .collect();
    assert_eq!(found, counted);
}

/// The drop of r3 at the only join word, the `mtctr` after `count`
/// paths.
fn r3_drop(count: u16, brought: Option<usize>) -> (usize, JoinDrop) {
    let join = BASE + 4 * 3 * u64::from(count);
    let register = JoinRegister::Gpr(3);
    (
        0,
        JoinDrop {
            join,
            register,
            brought,
        },
    )
}

#[test]
fn a_drop_counts_each_constant_that_the_paths_bring_to_its_join() {
    let found = drops(paths(17), &Bounds::default());
    assert_eq!(found, vec![r3_drop(17, Some(17))]);
}

#[test]
fn a_raised_bound_keeps_each_constant_that_the_paths_bring() {
    let mut bounds = Bounds::default();
    let first = drops(paths(17), &bounds);
    assert!(bounds.raise(once(&Origins::of(0)), &first));
    assert_eq!(drops(paths(17), &bounds), Vec::new());
}

#[test]
fn a_drop_past_the_raised_limit_has_no_count() {
    let found = drops(paths(65), &Bounds::default());
    assert_eq!(found, vec![r3_drop(65, None)]);
}

#[test]
fn a_raise_takes_the_count_or_double_the_bound_up_to_the_raised_limit_and_skips_no_count() {
    let mut bounds = Bounds::default();
    let r3 = JoinRegister::Gpr(3);
    let origins = Origins::of(0);
    assert!(bounds.raise(once(&origins), &[r3_drop(0, Some(17))]));
    assert_eq!(bounds.bound(0, r3), 32);
    assert!(bounds.raise(once(&origins), &[r3_drop(0, Some(33))]));
    assert_eq!(bounds.bound(0, r3), 64);
    assert!(!bounds.raise(once(&origins), &[r3_drop(0, None)]));
    assert_eq!(bounds.bound(0, r3), 64);
    let mut fresh = Bounds::default();
    assert!(fresh.raise(once(&origins), &[r3_drop(0, Some(40))]));
    assert_eq!(fresh.bound(0, r3), 40);
    assert_eq!(fresh.bound(0, JoinRegister::Lr), 16);
    // Twice 40 is past the raised limit, so the bound stops at 64.
    assert!(fresh.raise(once(&origins), &[r3_drop(0, Some(41))]));
    assert_eq!(fresh.bound(0, r3), 64);
    // A count at the bound raises nothing.
    assert!(!fresh.raise(once(&origins), &[r3_drop(0, Some(64))]));
    assert_eq!(fresh.bound(0, r3), 64);
}

#[test]
fn a_mark_keeps_the_sixteen_smallest_origins_and_has_more_past_them() {
    let joined = |count: u32| {
        (0..count)
            .rev()
            .map(Origins::of)
            .reduce(|one, two| one.union(&two))
            .expect("one origin or more")
    };
    let sixteen = joined(16);
    assert_eq!(sixteen.drops(), (0..16).collect::<Vec<u32>>());
    assert!(!sixteen.has_more());
    let seventeen = joined(17);
    assert_eq!(seventeen.drops(), (0..16).collect::<Vec<u32>>());
    assert!(seventeen.has_more());
}

#[test]
fn a_full_mark_takes_a_drop_above_its_origins_with_no_change() {
    let full = (0..17)
        .map(Origins::of)
        .reduce(|one, two| one.union(&two))
        .expect("one origin or more");
    let mut own = Some(full.clone());
    assert!(!join_origins(&mut own, Some(&Origins::of(40))));
    assert_eq!(own, Some(full.clone()));
    let mut lower = Some(Origins::of(40).union(&full));
    assert_eq!(lower, Some(full));
    assert!(!join_origins(&mut lower, Some(&Origins::of(16))));
}

#[test]
fn a_join_of_origins_changes_the_mark_only_for_an_origin_it_does_not_hold() {
    let mut own = None;
    assert!(join_origins(&mut own, Some(&Origins::of(3))));
    assert!(!join_origins(&mut own, Some(&Origins::of(3))));
    assert!(!join_origins(&mut own, None));
    assert!(join_origins(&mut own, Some(&Origins::of(1))));
    assert_eq!(own.as_ref().map(Origins::drops), Some(&[1, 3][..]));
}
