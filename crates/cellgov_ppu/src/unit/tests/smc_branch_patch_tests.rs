//! The self-modifying-code fixture pair: a loop patches a word of its
//! own body under `dcbst`, `sync`, `icbi`, `isync`, and the next pass
//! runs the new word.
//!
//! - Patch in: a terminator (a branch, or `sc`) replaces an ordinary
//!   instruction, so the loop stops on the next pass.
//! - Patch out: an ordinary instruction replaces a terminator, so the
//!   loop counts every later pass.
//! - Fused pair: the loop replaces either half of a fused `li; stw` pair.
//!
//! Each case runs with and without a predecoded shadow over the text. The
//! budgets end a batch on every instruction, inside the patch sequence,
//! and around the whole loop.
//!
//! [PPC-Book2 p:12 s:1.8] Case 1: after `dcbst`, `sync`, `icbi` and
//! `isync`, the next fetch of the stored-to location runs the new
//! instruction.

use crate::instruction::encode::encode;
use crate::instruction::PpuInstruction as I;
use crate::shadow::PredecodedShadow;
use crate::*;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome};
use cellgov_time::Budget;

/// The address of the first slot word.
const SLOT: u64 = 0x20;
/// The last loop counter the loop stored.
const RESULT: u64 = 0x300;
/// The number of passes that ran an `addi r5,r5,1` in the slot (r5 at exit).
const COUNT: u64 = 0x304;
/// The word the fused pair's store writes.
const PAIR: u64 = 0x308;
/// The word a patched second half stores to instead.
const PAIR2: u64 = 0x30C;
/// The pass on which the loop patches the slot.
const PATCH_PASS: i16 = 3;
/// The pass after which the loop falls through to the exit.
const LAST_PASS: i16 = 10;

fn word(insn: I) -> u32 {
    encode(&insn).expect("encodable")
}

fn addi(rt: u8, ra: u8, imm: i16) -> u32 {
    word(I::Addi { rt, ra, imm })
}

fn stw(rs: u8, ra: u8, imm: u64) -> u32 {
    word(I::Stw {
        rs,
        ra,
        imm: imm as i16,
    })
}

fn branch(from: u64, to: u64) -> u32 {
    word(I::B {
        offset: (to as i64 - from as i64) as i32,
        aa: false,
        link: false,
    })
}

fn bc(bo: u8, bi: u8, from: u64, to: u64) -> u32 {
    word(I::Bc {
        bo,
        bi,
        offset: (to as i64 - from as i64) as i16,
        aa: false,
        link: false,
    })
}

/// One fixture of the self-modifying loop.
struct Patch {
    slot: Vec<u32>,
    /// The index of the slot word that the loop patches.
    at: usize,
    /// The new word, given the patched pc and the exit's pc.
    new: fn(u64, u64) -> u32,
    /// The syscall number that r11 holds in the loop.
    r11: u64,
}

/// The program's words from address 0. The pass counter is r14, which a
/// system call in the slot leaves alone.
///
/// ```text
///         li r14,0 ; li r5,0 ; li r11,<r11>
///         r7 = <new word> ; r8 = <patched pc>
/// loop:   addi r14,r14,1
///         stw r14,RESULT
///         <slot>
///         cmpwi r14,PATCH_PASS ; bne skip
///         stw r7,0(r8) ; dcbst 0,r8 ; sync ; icbi 0,r8 ; isync
/// skip:   cmpwi r14,LAST_PASS ; blt loop
/// out:    stw r5,COUNT ; li r11,exit ; sc
/// ```
fn program(patch: &Patch) -> Vec<u32> {
    let at = |index: usize| 4 * index as u64;
    let loop_index = 6;
    let slot_index = 8;
    assert_eq!(at(slot_index), SLOT);
    let check = slot_index + patch.slot.len();
    let skip = check + 7;
    let out = skip + 2;
    let patched = SLOT + 4 * patch.at as u64;
    let new = (patch.new)(patched, at(out));
    let mut words = vec![
        addi(14, 0, 0),
        addi(5, 0, 0),
        addi(11, 0, patch.r11 as i16),
        word(I::Addis {
            rt: 7,
            ra: 0,
            imm: (new >> 16) as i16,
        }),
        word(I::Ori {
            ra: 7,
            rs: 7,
            imm: new as u16,
        }),
        addi(8, 0, patched as i16),
        addi(14, 14, 1),
        stw(14, 0, RESULT),
    ];
    words.extend(&patch.slot);
    words.extend([
        word(I::Cmpwi {
            bf: 0,
            ra: 14,
            imm: PATCH_PASS,
        }),
        bc(4, 2, at(check + 1), at(skip)),
        stw(7, 8, 0),
        word(I::Dcbst { ra: 0, rb: 8 }),
        word(I::Sync { l: 0 }),
        word(I::Icbi { ra: 0, rb: 8 }),
        word(I::Isync),
        word(I::Cmpwi {
            bf: 0,
            ra: 14,
            imm: LAST_PASS,
        }),
        bc(12, 0, at(skip + 1), at(loop_index)),
        stw(5, 0, COUNT),
        addi(11, 0, sc::PROCESS_EXIT as i16),
        word(I::Sc { lev: 0 }),
    ]);
    assert_eq!(words.len(), out + 3);
    words
}

/// The words at [`RESULT`], [`COUNT`], [`PAIR`] and [`PAIR2`] after the
/// program runs at `budget`, with or without a predecoded shadow.
fn run(patch: &Patch, budget: u64, shadowed: bool) -> [u32; 4] {
    let words = program(patch);
    let text: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    assert!(
        (text.len() as u64) <= RESULT,
        "the program ends below its results"
    );
    let shadow = shadowed.then(|| PredecodedShadow::build(0, &text));
    let fixture = ScenarioFixture::builder()
        .memory_size(0x400)
        .budget(Budget::new(budget))
        .max_steps(100_000)
        .seed_memory(move |mem| {
            let range = ByteRange::new(GuestAddr::new(0), text.len() as u64).expect("range");
            mem.apply_commit(range, &text).expect("mapped");
        })
        .register(move |rt| {
            rt.register_unit_with(move |id| {
                let mut ppu = PpuExecutionUnit::new(id);
                if let Some(shadow) = shadow {
                    ppu.set_instruction_shadow(shadow);
                }
                ppu
            });
        })
        .build();
    let result = runner::run(fixture);
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.first_fault, None);
    let read = |addr: u64| {
        let at = addr as usize;
        u32::from_be_bytes(result.final_memory[at..at + 4].try_into().expect("4"))
    };
    [read(RESULT), read(COUNT), read(PAIR), read(PAIR2)]
}

/// Runs `patch` under every budget, with and without a shadow, and hands
/// each result to `check`.
fn for_every_run(patch: &Patch, check: impl Fn([u32; 4], &str)) {
    for shadowed in [false, true] {
        for budget in [1, 2, 7, 256] {
            check(
                run(patch, budget, shadowed),
                &format!("budget {budget}, shadow {shadowed}"),
            );
        }
    }
}

/// The pass after the patch: the first one that runs the new word.
const NEXT_PASS: u32 = PATCH_PASS as u32 + 1;

#[test]
fn a_branch_patched_into_the_loop_exits_on_the_next_pass() {
    let patch = Patch {
        slot: vec![addi(5, 5, 1)],
        at: 0,
        new: branch,
        r11: 0,
    };
    for_every_run(&patch, |[result, count, _, _], case| {
        assert_eq!(result, NEXT_PASS, "{case}");
        assert_eq!(count, PATCH_PASS as u32, "{case}");
    });
}

#[test]
fn an_sc_patched_into_the_loop_exits_on_the_next_pass() {
    let patch = Patch {
        slot: vec![addi(5, 5, 1)],
        at: 0,
        new: |_, _| word(I::Sc { lev: 0 }),
        r11: sc::PROCESS_EXIT,
    };
    for_every_run(&patch, |[result, count, _, _], case| {
        assert_eq!(result, NEXT_PASS, "{case}");
        assert_eq!(count, 0, "{case}: the process exits before the exit path");
    });
}

#[test]
fn a_branch_patched_out_of_the_loop_lets_every_later_pass_count() {
    let patch = Patch {
        slot: vec![branch(SLOT, SLOT + 4)],
        at: 0,
        new: |_, _| addi(5, 5, 1),
        r11: 0,
    };
    for_every_run(&patch, |[result, count, _, _], case| {
        assert_eq!(result, LAST_PASS as u32, "{case}");
        assert_eq!(count, (LAST_PASS - PATCH_PASS) as u32, "{case}");
    });
}

#[test]
fn an_sc_patched_out_of_the_loop_lets_every_later_pass_count() {
    let patch = Patch {
        slot: vec![word(I::Sc { lev: 0 })],
        at: 0,
        new: |_, _| addi(5, 5, 1),
        r11: sc::PPU_THREAD_YIELD,
    };
    for_every_run(&patch, |[result, count, _, _], case| {
        assert_eq!(result, LAST_PASS as u32, "{case}");
        assert_eq!(count, (LAST_PASS - PATCH_PASS) as u32, "{case}");
    });
}

/// `li r6,7 ; stw r6,PAIR(r9)`, which the shadow fuses; r9 stays zero.
fn fused_li_stw() -> Vec<u32> {
    vec![addi(6, 0, 7), stw(6, 9, PAIR)]
}

#[test]
fn the_slot_pair_fuses_in_the_shadow() {
    let patch = Patch {
        slot: fused_li_stw(),
        at: 0,
        new: |_, _| addi(6, 0, 9),
        r11: 0,
    };
    let text: Vec<u8> = program(&patch)
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect();
    let shadow = PredecodedShadow::build(0, &text);
    assert_eq!(shadow.get(SLOT + 4), Some(I::Consumed));
}

#[test]
fn a_patched_head_of_a_fused_pair_runs_the_new_word() {
    let patch = Patch {
        slot: fused_li_stw(),
        at: 0,
        new: |_, _| addi(6, 0, 9),
        r11: 0,
    };
    for_every_run(&patch, |[result, _, pair, pair2], case| {
        assert_eq!(result, LAST_PASS as u32, "{case}");
        assert_eq!(pair, 9, "{case}: the later passes store the new value");
        assert_eq!(pair2, 0, "{case}");
    });
}

#[test]
fn a_patched_second_half_of_a_fused_pair_runs_the_new_word() {
    let patch = Patch {
        slot: fused_li_stw(),
        at: 1,
        new: |_, _| stw(6, 9, PAIR2),
        r11: 0,
    };
    for_every_run(&patch, |[result, _, pair, pair2], case| {
        assert_eq!(result, LAST_PASS as u32, "{case}");
        assert_eq!(pair, 7, "{case}");
        assert_eq!(
            pair2, 7,
            "{case}: the later passes store to the new address"
        );
    });
}
