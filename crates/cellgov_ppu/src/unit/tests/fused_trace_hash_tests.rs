//! Per-step tracing records the state after each instruction of a pair
//! the shadow fuses, as it does for the same two instructions unfused.

use crate::instruction::encode::encode;
use crate::instruction::PpuInstruction as I;
use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, PpuFingerprint};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

/// The word `lwz` loads.
const DATA: u64 = 0x40;

/// `lwz r9, DATA(0); cmpwi cr7, r9, 0; b .`, with 5 at [`DATA`].
fn program() -> (GuestMemory, Vec<u8>) {
    let words = [
        I::Lwz {
            rt: 9,
            ra: 0,
            imm: DATA as i16,
        },
        I::Cmpwi {
            bf: 7,
            ra: 9,
            imm: 0,
        },
        I::B {
            offset: 0,
            aa: false,
            link: false,
        },
    ];
    let code: Vec<u8> = words
        .iter()
        .flat_map(|insn| encode(insn).expect("encodable").to_be_bytes())
        .collect();
    let mut mem = GuestMemory::new(0x100);
    let range = ByteRange::new(GuestAddr::new(0), code.len() as u64).expect("range");
    mem.apply_commit(range, &code).expect("mapped");
    let data = ByteRange::new(GuestAddr::new(DATA), 4).expect("range");
    mem.apply_commit(data, &5u32.to_be_bytes()).expect("mapped");
    (mem, code)
}

/// A unit over [`program`], with a fused shadow when `fused`.
fn unit(code: &[u8], fused: bool) -> PpuExecutionUnit {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        let shadow = shadow::PredecodedShadow::build(0, code);
        assert_eq!(
            shadow.get(4),
            Some(I::Consumed),
            "precondition: the pair fuses"
        );
        unit.set_instruction_shadow(shadow);
    }
    unit
}

/// The per-step hashes of the pair and the branch after it.
fn traced_hashes(fused: bool) -> Vec<(u64, u64)> {
    let (mem, code) = program();
    let mut unit = unit(&code, fused);
    let ctx = ExecutionContext::new(&mem).with_trace_per_step(true);
    unit.run_until_yield(Budget::new(3), &ctx, &mut Vec::new());
    unit.drain_retired_state_hashes()
}

/// The full states the window records over the pair.
fn window_states(fused: bool) -> Vec<(u64, u64, PpuFingerprint)> {
    let (mem, code) = program();
    let mut unit = unit(&code, fused);
    unit.set_full_state_window(Some((0, 1)));
    let ctx = ExecutionContext::new(&mem);
    unit.run_until_yield(Budget::new(3), &ctx, &mut Vec::new());
    unit.drain_retired_state_full()
}

#[test]
fn a_fused_pair_hashes_the_state_after_each_instruction() {
    let unfused = traced_hashes(false);
    assert_eq!(unfused.len(), 3);
    assert_ne!(
        unfused[0].1, unfused[1].1,
        "the cmpwi moves CR, so the two hashes differ"
    );
    assert_eq!(traced_hashes(true), unfused);
}

#[test]
fn a_full_state_window_records_the_state_after_each_instruction_of_a_fused_pair() {
    let unfused = window_states(false);
    assert_eq!(unfused.len(), 2);
    assert_ne!(unfused[0].2, unfused[1].2);
    assert_eq!(window_states(true), unfused);
}
