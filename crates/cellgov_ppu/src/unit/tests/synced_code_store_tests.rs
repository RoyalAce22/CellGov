//! A thread that stores an instruction, runs `dcbst`, `sync`, `icbi`,
//! `isync` and branches to it runs the new instruction at every budget.
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

/// The patched word: `li r3,0` until the program stores `li r3,1`.
const PATCHED: u64 = 0x40;
/// Where the program stores the r3 the patched word set.
const RESULT: u64 = 0x80;

fn word(insn: I) -> u32 {
    encode(&insn).expect("encodable")
}

/// The program's words at their addresses. The program:
/// 1. stores `li r3,1` over `li r3,0` at [`PATCHED`];
/// 2. runs the sequence and branches there;
/// 3. stores r3 at [`RESULT`] and exits.
fn program() -> Vec<(u64, u32)> {
    let new = word(I::Addi {
        rt: 3,
        ra: 0,
        imm: 1,
    });
    let body = [
        word(I::Addis {
            rt: 5,
            ra: 0,
            imm: (new >> 16) as i16,
        }),
        word(I::Ori {
            ra: 5,
            rs: 5,
            imm: new as u16,
        }),
        word(I::Addi {
            rt: 6,
            ra: 0,
            imm: PATCHED as i16,
        }),
        word(I::Stw {
            rs: 5,
            ra: 6,
            imm: 0,
        }),
        word(I::Dcbst { ra: 0, rb: 6 }),
        word(I::Sync { l: 0 }),
        word(I::Icbi { ra: 0, rb: 6 }),
        word(I::Isync),
    ];
    let at = 4 * body.len() as u64;
    let mut words: Vec<(u64, u32)> = body
        .into_iter()
        .enumerate()
        .map(|(i, w)| (4 * i as u64, w))
        .collect();
    words.push((
        at,
        word(I::B {
            offset: (PATCHED - at) as i32,
            aa: false,
            link: false,
        }),
    ));
    let tail = [
        word(I::Addi {
            rt: 3,
            ra: 0,
            imm: 0,
        }),
        word(I::Stw {
            rs: 3,
            ra: 0,
            imm: RESULT as i16,
        }),
        word(I::Addi {
            rt: 11,
            ra: 0,
            imm: sc::PROCESS_EXIT as i16,
        }),
        word(I::Sc { lev: 0 }),
    ];
    words.extend(
        tail.into_iter()
            .enumerate()
            .map(|(i, w)| (PATCHED + 4 * i as u64, w)),
    );
    words
}

/// The r3 the patched word set, run at `budget` with or without a
/// predecoded shadow over the program's text.
fn run(budget: u64, shadowed: bool) -> u32 {
    let words = program();
    let text_end = words.iter().map(|&(at, _)| at + 4).max().expect("words");
    assert!(text_end <= RESULT, "the program ends below the result word");
    let mut text = vec![0u8; text_end as usize];
    for &(at, w) in &words {
        text[at as usize..at as usize + 4].copy_from_slice(&w.to_be_bytes());
    }
    let shadow = shadowed.then(|| PredecodedShadow::build(0, &text));
    let fixture = ScenarioFixture::builder()
        .memory_size(0x100)
        .budget(Budget::new(budget))
        .max_steps(10_000)
        .seed_memory(move |mem| {
            for (at, w) in words {
                let range = ByteRange::new(GuestAddr::new(at), 4).expect("range");
                mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
            }
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
    let at = RESULT as usize;
    u32::from_be_bytes(result.final_memory[at..at + 4].try_into().expect("4"))
}

#[test]
fn the_sequence_makes_a_same_batch_code_store_visible_at_every_budget() {
    for shadowed in [false, true] {
        for budget in [1, 2, 256] {
            assert_eq!(
                run(budget, shadowed),
                1,
                "budget {budget}, shadow {shadowed}"
            );
        }
    }
}
