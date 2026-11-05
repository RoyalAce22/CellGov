//! A doubleword store keeps its line in the patch class only when it is
//! atomic and swaps a patch-class word at each of its two code words.

use std::collections::VecDeque;

use cellgov_exec::CodeEvent;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use super::tests::{fetch, memory, step, Scripted, Step, NOP, X};
use crate::runtime::{AddressSpaceId, Runtime, UnsyncedCodeFetch};

/// The doubleword that holds `X` as its second word.
const D: u64 = X - 4;
const B_8: u32 = 0x4800_0008;
const LI_R3_1: u32 = 0x3860_0001;

/// A doubleword store at `D` of `hi` then `lo`.
fn store_d(hi: u32, lo: u32, atomic: bool) -> CodeEvent {
    CodeEvent::Store {
        addr: D,
        len: 8,
        value: u128::from(hi) << 32 | u128::from(lo),
        atomic,
    }
}

/// Runs one scripted unit over [`memory`] with `nop` at `D` and `b +8`
/// at `X`, and returns the records.
fn run(steps: &[Step]) -> Vec<UnsyncedCodeFetch> {
    run_over(memory(), true, steps)
}

/// Runs one scripted unit over `mem`, with `D` set to `nop` first when
/// `nop_at_d`, and returns the records.
fn run_over(mem: GuestMemory, nop_at_d: bool, steps: &[Step]) -> Vec<UnsyncedCodeFetch> {
    let mut rt = Runtime::new(mem, Budget::new(1), 1000);
    if nop_at_d {
        let range = ByteRange::new(GuestAddr::new(D), 4).unwrap();
        rt.place_bytes(AddressSpaceId::BOOT, range, &NOP.to_be_bytes())
            .expect("the placement lands");
    }
    let steps: VecDeque<Step> = steps.iter().cloned().collect();
    rt.register_unit_with(|id| Scripted {
        id,
        steps,
        drained: Vec::new(),
    });
    for _ in 0..8 {
        let Ok(mut s) = rt.step() else { break };
        rt.commit_step(&mut s).expect("the batch commits");
    }
    rt.unsynced_code_fetches().to_vec()
}

fn patch_class(store: CodeEvent) -> bool {
    only_record(run(&[step(&[store, fetch(X)])]))
}

fn only_record(records: Vec<UnsyncedCodeFetch>) -> bool {
    let [record] = records[..] else {
        panic!("one record, got {records:?}");
    };
    record.patch_class
}

#[test]
fn an_atomic_doubleword_that_swaps_both_patch_words_is_patch_class() {
    assert!(patch_class(store_d(B_8, NOP, true)));
}

#[test]
fn a_doubleword_that_leaves_the_patch_class_at_either_word_is_not_patch_class() {
    assert!(!patch_class(store_d(LI_R3_1, NOP, true)), "first word");
    assert!(!patch_class(store_d(B_8, LI_R3_1, true)), "second word");
}

#[test]
fn a_doubleword_that_is_not_atomic_is_not_patch_class() {
    assert!(!patch_class(store_d(B_8, NOP, false)));
}

#[test]
fn a_doubleword_word_outside_code_does_not_decide_the_patch_class() {
    // Code starts at `X`, so `D` is data and its new word is not code.
    let mut mem = GuestMemory::new(0x400);
    let range = ByteRange::new(GuestAddr::new(X), 4).unwrap();
    mem.apply_commit(range, &B_8.to_be_bytes()).unwrap();
    mem.declare_code(X, 0x200 - X);
    let records = run_over(
        mem,
        false,
        &[step(&[store_d(LI_R3_1, NOP, true), fetch(X)])],
    );
    assert!(only_record(records));
}

#[test]
fn a_doubleword_replaces_the_word_an_earlier_word_store_in_its_batch_wrote() {
    // The sequence makes the line consistent, so the doubleword starts a
    // new mark whose old word at `X` is the `li` stored before it.
    let records = run(&[step(&[
        super::tests::store(X, LI_R3_1),
        CodeEvent::Dcbst { addr: X },
        CodeEvent::Sync,
        CodeEvent::Icbi { addr: X },
        CodeEvent::Isync,
        store_d(NOP, NOP, true),
        fetch(X),
    ])]);
    assert!(!only_record(records));
}
