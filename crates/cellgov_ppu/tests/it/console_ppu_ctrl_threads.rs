//! CellGov's PPU against the `ppu_ctrl_threads` console capture: the CTRL
//! value a thread reads alone and beside a sibling.
//!
//! Three console threads read CTRL at once and record each distinct value.
//! The replay reads CTRL under each PPE placement CellGov forms, and from
//! two threads that run in one runtime, and holds every value to the set
//! the console read.

use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/ppu_ctrl_threads/ps3/cech20-cex-493/observation.json";

/// The payload's word 37 when the probe wrote every word.
const DONE: u64 = 0x4354_5254;

/// CTRL TE, bits 8:9.
const TE: u64 = 0x00C0_0000;

/// Every distinct CTRL value the three console threads read.
fn console_values() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 304);
    let w: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[37], DONE, "the probe wrote every word");
    let mut values = Vec::new();
    for k in 0..3 {
        let b = 12 * k;
        assert_eq!((w[b], w[b + 1]), (0, 0), "reader {k} created and joined");
        assert!(w[b + 3] > 0, "reader {k} read CTRL");
        for i in 0..4 {
            if w[b + 8 + i] != 0 && !values.contains(&w[b + 4 + i]) {
                values.push(w[b + 4 + i]);
            }
        }
    }
    values
}

/// The CTRL a thread reads with `siblings` PPE threads beside it, one of
/// them created first when `older`.
fn cellgov_ctrl(siblings: u32, older: bool) -> u64 {
    const CODE: u64 = 0x1000;
    let mut mem = GuestMemory::new(0x2000);
    for (i, w) in [0x7C68_22A6u32, 0x4800_0000].iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(CODE + 4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = CODE;
    let ctx = ExecutionContext::new(&mem)
        .with_ppe_siblings(siblings)
        .with_ppe_older_sibling(older);
    unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());
    assert_eq!(unit.state().pc, CODE + 4, "mfspr r3, 136 retired");
    unit.state().gpr[3]
}

#[test]
fn each_placement_reads_a_ctrl_value_the_console_read() {
    let console = console_values();
    let alone = cellgov_ctrl(0, false);
    let first = cellgov_ctrl(1, false);
    let second = cellgov_ctrl(1, true);
    for (placement, v) in [("alone", alone), ("first", first), ("second", second)] {
        assert!(
            console.contains(&v),
            "{placement}: 0x{v:08x} not among {console:x?}"
        );
    }
    assert_ne!(alone & TE, TE, "alone: one thread enabled");
    assert_eq!((first & TE, second & TE), (TE, TE), "both threads enabled");
    assert_ne!(first >> 30, second >> 30, "the two threads differ in CT");
}

/// The CTRL each of two PPU threads reads when both run in one runtime,
/// in the order the threads were created.
fn runtime_ctrl_pair() -> [u64; 2] {
    const CODE: u64 = 0x1000;
    let mut mem = GuestMemory::new(0x2000);
    for (i, w) in [0x7C68_22A6u32, 0x4800_0000].iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(CODE + 4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    let mut rt = cellgov_core::Runtime::new(mem, Budget::new(1), 10_000);
    let mut thread = || {
        rt.register_unit_with(|id| {
            let mut ppu = PpuExecutionUnit::new(id);
            ppu.state_mut().pc = CODE;
            ppu
        })
    };
    let ids = [thread(), thread()];
    let state = |rt: &cellgov_core::Runtime, id| {
        rt.registry()
            .get(id)
            .and_then(|u| u.as_any().downcast_ref::<PpuExecutionUnit>())
            .expect("the PPU unit")
            .state()
            .clone()
    };
    for _ in 0..8 {
        if ids.iter().all(|&id| state(&rt, id).pc == CODE + 4) {
            break;
        }
        let mut step = rt.step().expect("a runnable PPU thread");
        rt.commit_step(&mut step).expect("the step commits");
    }
    ids.map(|id| {
        let s = state(&rt, id);
        assert_eq!(s.pc, CODE + 4, "mfspr r3, 136 retired");
        s.gpr[3]
    })
}

#[test]
fn two_ppu_threads_in_one_runtime_read_console_ctrl_values_with_distinct_ct() {
    let console = console_values();
    let [first, second] = runtime_ctrl_pair();
    for (placement, v) in [("first", first), ("second", second)] {
        assert!(
            console.contains(&v),
            "{placement}: 0x{v:08x} not among {console:x?}"
        );
        assert_eq!(v & TE, TE, "{placement}: both threads enabled");
    }
    assert_ne!(first >> 30, second >> 30, "the two threads differ in CT");
}
