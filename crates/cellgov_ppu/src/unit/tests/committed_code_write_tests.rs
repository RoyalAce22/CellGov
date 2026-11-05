//! A write that commits outside a plain store still reaches the next
//! fetch of a predecoded word:
//!
//! - a successful `stwcx.` under the modification sequence;
//! - bytes the driver places over code.
//!
//! [PPC-Book2 p:12 s:1.8] Case 1: after `dcbst`, `sync`, `icbi` and
//! `isync`, the next fetch of the stored-to location runs the new
//! instruction.

use crate::instruction::encode::encode;
use crate::instruction::PpuInstruction as I;
use crate::shadow::PredecodedShadow;
use crate::*;
use cellgov_core::AddressSpaceId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome};
use cellgov_time::Budget;

/// The patched word: `li r3,0` until something writes `li r3,1`.
const PATCHED: u64 = 0x40;
/// Where the program stores the r3 the patched word set.
const RESULT: u64 = 0x80;

fn word(insn: I) -> u32 {
    encode(&insn).expect("encodable")
}

/// `li r3,1`, the word every test writes over [`PATCHED`].
fn new_word() -> u32 {
    word(I::Addi {
        rt: 3,
        ra: 0,
        imm: 1,
    })
}

/// The program's words at their addresses: `body` from address 0, a
/// branch to [`PATCHED`], then `li r3,0`, a store of r3 at [`RESULT`]
/// and an exit.
fn program(body: &[u32]) -> Vec<(u64, u32)> {
    let at = 4 * body.len() as u64;
    let mut words: Vec<(u64, u32)> = body
        .iter()
        .enumerate()
        .map(|(i, &w)| (4 * i as u64, w))
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

/// A `lwarx` / `stwcx.` pair that stores [`new_word`] over [`PATCHED`],
/// then `dcbst`, `sync`, `icbi` and `isync`.
fn conditional_store_body() -> Vec<u32> {
    let new = new_word();
    vec![
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
        word(I::Lwarx {
            rt: 7,
            ra: 0,
            rb: 6,
        }),
        word(I::Stwcx {
            rs: 5,
            ra: 0,
            rb: 6,
        }),
        word(I::Dcbst { ra: 0, rb: 6 }),
        word(I::Sync { l: 0 }),
        word(I::Icbi { ra: 0, rb: 6 }),
        word(I::Isync),
    ]
}

/// The r3 the patched word set, run at `budget` with or without a
/// predecoded shadow over the program's text. With `placed`, the driver
/// places [`new_word`] over [`PATCHED`] after the shadow is built and
/// before the first step.
fn run(body: &[u32], budget: u64, shadowed: bool, placed: bool) -> u32 {
    let words = program(body);
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
            if placed {
                let range = ByteRange::new(GuestAddr::new(PATCHED), 4).expect("range");
                rt.place_bytes(AddressSpaceId::BOOT, range, &new_word().to_be_bytes())
                    .expect("the placement lands in mapped text");
            }
        })
        .build();
    let result = runner::run(fixture);
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.first_fault, None);
    let at = RESULT as usize;
    u32::from_be_bytes(result.final_memory[at..at + 4].try_into().expect("4"))
}

#[test]
fn a_successful_stwcx_over_predecoded_code_runs_the_stored_word_after_the_sequence() {
    let body = conditional_store_body();
    for shadowed in [false, true] {
        for budget in [1, 2, 256] {
            assert_eq!(
                run(&body, budget, shadowed, false),
                1,
                "budget {budget}, shadow {shadowed}"
            );
        }
    }
}

#[test]
fn bytes_placed_over_predecoded_code_run_in_place_of_the_decoded_word() {
    for shadowed in [false, true] {
        for budget in [1, 256] {
            assert_eq!(
                run(&[], budget, shadowed, true),
                1,
                "budget {budget}, shadow {shadowed}"
            );
        }
    }
}
