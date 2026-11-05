//! What the caches remember about a touched, flushed or stored-back line,
//! and how long a later load of it waits.

use super::cache::{Caches, Level};
use super::{classify, PpeTiming};
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu_issue as k;

/// A line no test loads before it starts.
const LINE_A: u64 = 0x100_0000;

/// As many lines as the L2 holds: more than any table of pending lines.
const OVERFILL: u64 = (k::PPU_L2_SETS * k::PPU_L2_WAYS) as u64;

#[test]
fn a_pending_flush_survives_more_later_store_backs_than_the_table_holds() {
    let mut caches = Caches::new();
    caches.flush(0, LINE_A);
    for i in 1..OVERFILL {
        caches.store_back(0, LINE_A + i * k::PPU_CACHE_LINE_BYTES);
    }
    caches.store_back(10, LINE_A + OVERFILL * k::PPU_CACHE_LINE_BYTES);
    assert_eq!(caches.busy(10, LINE_A, 8), k::PPU_DCBF_LINE_BUSY_CYCLES);
}

#[test]
fn a_demand_reload_from_memory_ends_the_touch_of_its_line() {
    let mut caches = Caches::new();
    caches.touch(0, LINE_A);
    // Lines that share the touched line's L2 set push it out of both caches.
    let stride = k::PPU_L2_SETS as u64 * k::PPU_CACHE_LINE_BYTES;
    for j in 1..=2 * k::PPU_L2_WAYS as u64 {
        caches.load(LINE_A + j * stride, 8, false);
    }
    assert_eq!(caches.load(LINE_A, 8, false), Level::Memory);
    assert_eq!(caches.arrival(100, LINE_A, 8), 0);
}

#[test]
fn a_load_of_a_touched_line_waits_no_longer_than_its_own_trip_to_memory() {
    let mut timing = PpeTiming::new();
    let mut state = PpuState::new();
    // Eight touches of eight lines queue one request every
    // PPU_TOUCH_INTERVAL_CYCLES; the load reads the last line.
    let mut insns = Vec::new();
    for i in 0..8u8 {
        state.set_gpr(
            usize::from(10 + i),
            LINE_A + u64::from(i) * k::PPU_CACHE_LINE_BYTES,
        );
        insns.push(I::Dcbt {
            ra: 0,
            rb: 10 + i,
            th: 0,
        });
    }
    insns.push(I::Ld {
        rt: 20,
        ra: 17,
        imm: 0,
    });
    insns.push(I::Add {
        rt: 21,
        ra: 20,
        rb: 20,
        oe: false,
        rc: false,
    });
    let mut cycles = Vec::new();
    for (i, insn) in insns.iter().enumerate() {
        let at = 0x1000 + 4 * i as u64;
        let (ops, n) = classify(insn, &state);
        let issued = timing.retire_instruction(at, &ops[..n], at + 4);
        cycles.extend_from_slice(&issued[..n]);
    }
    assert_eq!(cycles[9], cycles[8] + k::PPU_MEMORY_LOAD_USE_CYCLES);
}
