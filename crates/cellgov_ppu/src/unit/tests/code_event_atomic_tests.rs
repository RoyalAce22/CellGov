//! A store event carries its bytes, and is atomic only when the store is
//! one aligned word or doubleword of a single-register store.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{CodeEvent, ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

const VALUE: u64 = 0x4800_0008_6000_0000;

/// `std r3, ds(0)`.
fn std_r3(ds: u16) -> u32 {
    (62 << 26) | (3 << 21) | u32::from(ds)
}

/// `stmw rs, 128(0)`.
fn stmw_at_128(rs: u32) -> u32 {
    (47 << 26) | (rs << 21) | 128
}

/// `stvx v2, 0, r4`.
fn stvx_v2_at_r4() -> u32 {
    (31 << 26) | (2 << 21) | (4 << 11) | (231 << 1)
}

/// Runs the one instruction `insn` from address 0 and returns the store
/// events it left.
///
/// The unit starts with:
///
/// - `VALUE` in r3, r30, r31 and both doublewords of v2;
/// - 128 in r4.
fn stores(insn: u32) -> Vec<CodeEvent> {
    let mut mem = GuestMemory::new(256);
    let range = ByteRange::new(GuestAddr::new(0), 4).unwrap();
    mem.apply_commit(range, &insn.to_be_bytes()).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    for r in [3, 30, 31] {
        unit.state_mut().set_gpr(r, VALUE);
    }
    unit.state_mut().set_gpr(4, 128);
    unit.state_mut()
        .set_vr(2, u128::from(VALUE) << 64 | u128::from(VALUE));
    let ctx = ExecutionContext::new(&mem).with_trace_per_step(true);
    unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());
    unit.drain_code_events()
        .into_iter()
        .filter(|e| matches!(e, CodeEvent::Store { .. }))
        .collect()
}

fn doubleword_at(addr: u64, atomic: bool) -> CodeEvent {
    CodeEvent::Store {
        addr,
        len: 8,
        value: u128::from(VALUE),
        atomic,
    }
}

#[test]
fn an_aligned_std_is_atomic() {
    assert_eq!(stores(std_r3(128)), [doubleword_at(128, true)]);
}

#[test]
fn a_std_off_its_doubleword_boundary_is_not_atomic() {
    assert_eq!(stores(std_r3(132)), [doubleword_at(132, false)]);
}

#[test]
fn no_word_of_a_stmw_is_atomic() {
    // A store-multiple stores the low word of each register.
    let word = |addr| CodeEvent::Store {
        addr,
        len: 4,
        value: u128::from(VALUE as u32),
        atomic: false,
    };
    assert_eq!(stores(stmw_at_128(30)), [word(128), word(132)]);
    assert_eq!(stores(stmw_at_128(31)), [word(128)], "one register");
}

#[test]
fn no_doubleword_of_a_quadword_stvx_is_atomic() {
    // [PPC-Book2 p:3 s:1.4] a quadword is no atomic access width.
    assert_eq!(
        stores(stvx_v2_at_r4()),
        [doubleword_at(128, false), doubleword_at(136, false)]
    );
}
