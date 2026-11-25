//! Two PPU threads that hold clones of one predecoded shadow: one thread
//! patches a word the other has run, and the other then runs the new
//! word.
//!
//! [PPC-Book2 p:12 s:1.8] Case 2: the modifying program runs `dcbst`,
//! `sync`, `icbi` and `sync` and then sets a flag; the waiting program
//! loops until it loads the flag, runs `isync`, and then runs the new
//! instruction.

use crate::instruction::encode::encode;
use crate::instruction::PpuInstruction as I;
use crate::shadow::PredecodedShadow;
use crate::PpuExecutionUnit;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome};
use cellgov_time::Budget;

/// The first word of the patching thread.
const PATCHER: u64 = 0x000;
/// The first word of the thread that runs the patched word.
const RUNNER: u64 = 0x100;
/// The subroutine `li r3,1 ; blr` whose first word the patcher rewrites.
const TARGET: u64 = 0x200;
/// The runner's flag: it ran the subroutine once.
const READY: u64 = 0x300;
/// The patcher's flag: the new word is in place.
const DONE: u64 = 0x304;
/// The r3 that the runner's first call returns.
const FIRST: u64 = 0x308;
/// The r3 that the runner's second call returns.
const SECOND: u64 = 0x30C;

fn word(insn: I) -> u32 {
    encode(&insn).expect("encodable")
}

fn li(rt: u8, imm: i16) -> u32 {
    word(I::Addi { rt, ra: 0, imm })
}

fn lwz(rt: u8, at: u64) -> u32 {
    word(I::Lwz {
        rt,
        ra: 0,
        imm: at as i16,
    })
}

fn stw(rs: u8, at: u64) -> u32 {
    word(I::Stw {
        rs,
        ra: 0,
        imm: at as i16,
    })
}

fn cmpwi_1(ra: u8) -> u32 {
    word(I::Cmpwi { bf: 0, ra, imm: 1 })
}

fn bne(from: u64, to: u64) -> u32 {
    word(I::Bc {
        bo: 4,
        bi: 2,
        offset: (to as i64 - from as i64) as i16,
        aa: false,
        link: false,
    })
}

fn b(from: u64, to: u64, link: bool) -> u32 {
    word(I::B {
        offset: (to as i64 - from as i64) as i32,
        aa: false,
        link,
    })
}

fn place(text: &mut [u8], at: u64, words: &[u32]) {
    for (i, w) in words.iter().enumerate() {
        let off = at as usize + 4 * i;
        text[off..off + 4].copy_from_slice(&w.to_be_bytes());
    }
}

/// The text of both threads and the subroutine.
///
/// ```text
/// patcher: r7 = li r3,2 ; r8 = TARGET
///   wait:  lwz r5,READY ; cmpwi r5,1 ; bne wait
///          stw r7,0(r8) ; dcbst 0,r8 ; sync ; icbi 0,r8 ; sync
///          li r6,1 ; stw r6,DONE
///   spin:  b spin
/// runner:  bl TARGET ; stw r3,FIRST ; li r4,1 ; stw r4,READY
///   wait:  lwz r5,DONE ; cmpwi r5,1 ; bne wait ; isync
///          bl TARGET ; stw r3,SECOND ; li r11,exit ; sc
/// TARGET:  li r3,1 ; blr
/// ```
fn text() -> Vec<u8> {
    let new = li(3, 2);
    let mut text = vec![0u8; TARGET as usize + 8];
    let wait = PATCHER + 12;
    let spin = PATCHER + 0x34;
    place(
        &mut text,
        PATCHER,
        &[
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
            li(8, TARGET as i16),
            lwz(5, READY),
            cmpwi_1(5),
            bne(wait + 8, wait),
            word(I::Stw {
                rs: 7,
                ra: 8,
                imm: 0,
            }),
            word(I::Dcbst { ra: 0, rb: 8 }),
            word(I::Sync { l: 0 }),
            word(I::Icbi { ra: 0, rb: 8 }),
            word(I::Sync { l: 0 }),
            li(6, 1),
            stw(6, DONE),
            b(spin, spin, false),
        ],
    );
    let wait = RUNNER + 0x10;
    place(
        &mut text,
        RUNNER,
        &[
            b(RUNNER, TARGET, true),
            stw(3, FIRST),
            li(4, 1),
            stw(4, READY),
            lwz(5, DONE),
            cmpwi_1(5),
            bne(wait + 8, wait),
            word(I::Isync),
            b(wait + 16, TARGET, true),
            stw(3, SECOND),
            li(11, sc::PROCESS_EXIT as i16),
            word(I::Sc { lev: 0 }),
        ],
    );
    place(
        &mut text,
        TARGET,
        &[
            li(3, 1),
            word(I::Bclr {
                bo: 20,
                bi: 0,
                link: false,
            }),
        ],
    );
    text
}

/// The words at [`FIRST`] and [`SECOND`] after both threads run at
/// `budget`, each thread with a clone of one shadow.
fn run(budget: u64) -> [u32; 2] {
    let text = text();
    let shadow = PredecodedShadow::build(0, &text);
    let fixture = ScenarioFixture::builder()
        .memory_size(0x400)
        .budget(Budget::new(budget))
        .max_steps(100_000)
        .seed_memory(move |mem| {
            let range = ByteRange::new(GuestAddr::new(0), text.len() as u64).expect("range");
            mem.apply_commit(range, &text).expect("mapped");
        })
        .register(move |rt| {
            for pc in [PATCHER, RUNNER] {
                let shadow = shadow.clone();
                rt.register_unit_with(move |id| {
                    let mut ppu = PpuExecutionUnit::new(id);
                    ppu.state_mut().pc = pc;
                    ppu.set_instruction_shadow(shadow);
                    ppu
                });
            }
        })
        .build();
    let result = runner::run(fixture);
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.first_fault, None);
    let read = |addr: u64| {
        let at = addr as usize;
        u32::from_be_bytes(result.final_memory[at..at + 4].try_into().expect("4"))
    };
    [read(FIRST), read(SECOND)]
}

#[test]
fn a_word_one_thread_patches_runs_new_in_a_thread_sharing_its_shadow() {
    for budget in [1, 2, 7, 256] {
        assert_eq!(run(budget), [1, 2], "budget {budget}");
    }
}
