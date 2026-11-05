//! The run's one time base: a read takes the higher of its own issue
//! tick's time base and the highest any unit read before the step.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu::CELL_PPU_TIMEBASE_HZ;
use cellgov_time::{Budget, GuestTicks, TICKS_PER_SECOND, TICKS_PER_TB_TICK};

/// `mftb rT`.
fn mftb(rt: u32) -> u32 {
    (31 << 26) | (rt << 21) | (12 << 16) | (8 << 11) | (371 << 1)
}

/// `mftbu rT`.
fn mftbu(rt: u32) -> u32 {
    (31 << 26) | (rt << 21) | (13 << 16) | (8 << 11) | (371 << 1)
}

/// `words` run as one step at `tick` with `time_base` the highest read
/// so far.
fn run_reads(words: &[u32], tick: u64, time_base: u64) -> PpuExecutionUnit {
    let mut mem = GuestMemory::new(64);
    for (i, w) in words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &w.to_be_bytes()).unwrap();
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem)
        .with_current_tick(GuestTicks::new(tick))
        .with_time_base(time_base);
    unit.run_until_yield(Budget::new(words.len() as u64), &ctx, &mut Vec::new());
    unit
}

/// `mftb r3`, then `mftb r4`, through [`run_reads`].
fn two_reads(tick: u64, time_base: u64) -> PpuExecutionUnit {
    run_reads(&[mftb(3), mftb(4)], tick, time_base)
}

#[test]
fn a_read_behind_the_run_s_time_base_returns_the_run_s() {
    let unit = two_reads(0, 500);
    let s = unit.state();
    assert_eq!((s.gpr[3], s.gpr[4]), (500, 500));
    assert_eq!(unit.time_base_read(), Some(500));
}

#[test]
fn a_read_ahead_of_the_run_s_time_base_returns_its_own() {
    let unit = two_reads(TICKS_PER_SECOND, 5);
    let s = unit.state();
    assert_eq!(s.gpr[3], CELL_PPU_TIMEBASE_HZ);
    assert_eq!(
        s.gpr[4],
        CELL_PPU_TIMEBASE_HZ + 1,
        "the second read issues one tick later"
    );
    assert_eq!(unit.time_base_read(), Some(CELL_PPU_TIMEBASE_HZ + 1));
}

#[test]
fn an_mftbu_mftb_mftbu_triple_across_a_low_word_carry_steps_the_high_word_by_one() {
    // [PPC-Book2 p:30 s:4.1] mftbu places TB bits 0:31 in RT and zeros the
    // high half of RT. The first read lands one tick before the carry
    // out of the low word; each read issues one tick after the one before.
    let tick = 0xFFFF_FFFF * TICKS_PER_TB_TICK;
    let unit = run_reads(&[mftbu(3), mftb(4), mftbu(5)], tick, 0);
    let s = unit.state();
    assert_eq!((s.gpr[3], s.gpr[4], s.gpr[5]), (0, 1 << 32, 1));
}
