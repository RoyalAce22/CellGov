//! An `sc` whose reserved LEV bits or reserved bit 31 are set is an
//! ordinary system call.
//!
//! [PPC-Book3 p:12 s:2.3.1] bits 0:5 of the LEV field are a reserved
//! field, and [PPC-Book1 p:3 s:1.5.2] the processor ignores a reserved
//! field.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

/// The yield reason and LEV of one `sc` word run from pc 0.
fn run_sc(raw: u32) -> (YieldReason, Option<u8>) {
    let mut mem = GuestMemory::new(0x100);
    let range = ByteRange::new(GuestAddr::new(0), 4).expect("range");
    mem.apply_commit(range, &raw.to_be_bytes()).expect("mapped");
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(4), &ctx, &mut Vec::new());
    (result.yield_reason, result.local_diagnostics.syscall_lev)
}

#[test]
fn every_even_lev_runs_a_system_call() {
    for lev in (0u32..=0x7F).step_by(2) {
        for bit31 in [0u32, 1] {
            let raw = 0x4400_0002 | (lev << 5) | bit31;
            assert_eq!(run_sc(raw), (YieldReason::Syscall, Some(0)), "0x{raw:08x}");
        }
    }
}

/// Runs `li r3,-1 ; li r11,getpid ; <sc> ; stw r3,PID ; li r11,exit ;
/// <sc>` to its exit with `sc_word` as each `sc`, and returns the word
/// at `PID`.
fn getpid_through(sc_word: u32) -> u32 {
    use crate::instruction::encode::encode;
    use crate::instruction::PpuInstruction as I;
    use cellgov_ps3_abi::lv2::syscall as sc;
    use cellgov_testkit::fixtures::ScenarioFixture;
    use cellgov_testkit::runner::{self, ScenarioOutcome};

    const PID: u64 = 0x80;
    let word = |insn: I| encode(&insn).expect("encodable");
    let words = [
        word(I::Addi {
            rt: 3,
            ra: 0,
            imm: -1,
        }),
        word(I::Addi {
            rt: 11,
            ra: 0,
            imm: sc::PROCESS_GETPID as i16,
        }),
        sc_word,
        word(I::Stw {
            rs: 3,
            ra: 0,
            imm: PID as i16,
        }),
        word(I::Addi {
            rt: 11,
            ra: 0,
            imm: sc::PROCESS_EXIT as i16,
        }),
        sc_word,
    ];
    let text: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let fixture = ScenarioFixture::builder()
        .memory_size(0x100)
        .budget(Budget::new(256))
        .max_steps(1_000)
        .seed_memory(move |mem| {
            let range = ByteRange::new(GuestAddr::new(0), text.len() as u64).expect("range");
            mem.apply_commit(range, &text).expect("mapped");
        })
        .register(|rt| {
            rt.register_unit_with(PpuExecutionUnit::new);
        })
        .build();
    let result = runner::run(fixture);
    assert_eq!(result.outcome, ScenarioOutcome::Stalled, "0x{sc_word:08x}");
    assert_eq!(result.first_fault, None, "0x{sc_word:08x}");
    let at = PID as usize;
    u32::from_be_bytes(result.final_memory[at..at + 4].try_into().expect("4"))
}

#[test]
fn an_sc_with_reserved_bits_set_runs_the_system_call_r11_names() {
    let plain = getpid_through(0x4400_0002);
    assert_ne!(plain, u32::MAX, "getpid wrote r3");
    // LEV = 2 and bit 31 set: both reserved.
    assert_eq!(getpid_through(0x4400_0043), plain);
}

#[test]
fn an_sc_word_classifies_as_a_hypercall_exactly_when_lev_bit_26_is_set() {
    use cellgov_lv2::request::{classify_with_lev, Lv2Request};
    use cellgov_ps3_abi::lv2::syscall as sc;
    let mut checked = 0;
    for field in 0u32..=0x7F {
        for bit31 in [0u32, 1] {
            let raw = 0x4400_0002 | (field << 5) | bit31;
            let Ok(crate::instruction::PpuInstruction::Sc { lev }) = crate::decode::decode(raw)
            else {
                panic!("0x{raw:08x} decodes as sc");
            };
            let request = classify_with_lev(lev, sc::PROCESS_GETPID, &[0; 8]);
            assert_eq!(
                matches!(request, Lv2Request::Hypercall { .. }),
                field & 1 == 1,
                "0x{raw:08x}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 256);
}

#[test]
fn every_odd_lev_stops_the_thread() {
    for lev in (1u32..=0x7F).step_by(2) {
        let raw = 0x4400_0002 | (lev << 5);
        assert_eq!(run_sc(raw), (YieldReason::Fault, None), "0x{raw:08x}");
    }
}
