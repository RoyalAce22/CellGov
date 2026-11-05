//! CellGov's PPU timing against the `ppu_issue_sector_cross` console
//! capture: integer loads whose bytes cross from one 32-byte L1 sector
//! into the next inside one line.

use cellgov_core::Runtime;
use cellgov_exec::UnitStatus;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_issue::{PPU_CACHE_LINE_BYTES, PPU_CYCLES_PER_TB_TICK};
use cellgov_time::Budget;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_sector_cross/ps3/cech20-cex-493/observation.json";

/// The payload's word 66 when the probe wrote every word.
const DONE: u64 = 0x5050_5358;

/// The probe's loops.
const LOOPS: usize = 16;

/// The bytes of one node: one line.
const NODE: u64 = PPU_CACHE_LINE_BYTES;

/// The nodes of a loop's ring.
const NODES: u64 = 64;

/// The load in a loop's body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Load {
    /// `ldx r9,r9,r5`.
    Ldx,
    /// `lwzx r9,r9,r5`.
    Lwzx,
    /// `lhzx r8,r9,r5; add r9,r3,r8`.
    Lhzx,
}

impl Load {
    /// The bytes the load reads.
    const fn bytes(self) -> usize {
        match self {
            Load::Ldx => 8,
            Load::Lwzx => 4,
            Load::Lhzx => 2,
        }
    }
}

/// Each loop's load and offset into the node, in the probe's order.
const PROBE: [(Load, u16); LOOPS] = [
    (Load::Ldx, 80),
    (Load::Ldx, 88),
    (Load::Ldx, 92),
    (Load::Ldx, 89),
    (Load::Ldx, 90),
    (Load::Ldx, 93),
    (Load::Ldx, 95),
    (Load::Lwzx, 80),
    (Load::Lwzx, 90),
    (Load::Lwzx, 94),
    (Load::Lwzx, 93),
    (Load::Lwzx, 95),
    (Load::Lhzx, 80),
    (Load::Lhzx, 94),
    (Load::Lhzx, 95),
    (Load::Lhzx, 93),
];

/// One loop of the console's payload.
#[derive(Clone, Copy, Debug)]
struct Console {
    /// The fastest run, in core cycles an iteration.
    fastest: f64,
    /// The slowest run, in core cycles an iteration.
    slowest: f64,
    /// The effective address of the loop's slot.
    slot: u64,
}

fn console() -> Vec<Console> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 536);
    let w: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[66], DONE, "the probe wrote every word");
    (0..LOOPS)
        .map(|l| {
            let n = w[48 + l] as f64;
            let per = |ticks: u64| (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / n;
            Console {
                fastest: per(w[2 * l]),
                slowest: per(w[2 * l + 1]),
                slot: w[32 + l],
            }
        })
        .collect()
}

/// Where the replay places its code.
const CODE: u64 = 0x1000;

/// Where the replay places a slot: the console's slot at the same place
/// in a 64 KB span, which keeps its L1 sets.
fn slot_at(console_slot: u64) -> u64 {
    0x10_0000 + console_slot % 0x1_0000
}

/// Iterations the replay times. It runs as many before them, which leave
/// the ring in the L1 as the probe's earlier runs do.
const ITERATIONS: u64 = 512;

/// The words of a loop, and the index of its timed loop's head. r3 holds
/// the slot, r4 the count and r5 the offset.
fn program(load: Load) -> (Vec<u32>, usize) {
    let mut w = vec![0x7c69_1b78, 0x7c89_03a6]; // mr r9,r3; mtctr r4
    let head = w.len();
    match load {
        Load::Ldx => w.push(0x7d29_282a),
        Load::Lwzx => w.push(0x7d29_282e),
        Load::Lhzx => w.extend([0x7d09_2a2e, 0x7d23_4214]),
    }
    let disp = (head as i64 - w.len() as i64) * 4;
    w.push(0x4200_0000 | (disp as u32 & 0xfffc)); // bdnz head
    (w, head)
}

/// Writes the ring at `slot`: each node holds, at `at`, the next node's
/// address, or for a halfword its offset from the slot.
fn link_ring(mem: &mut GuestMemory, slot: u64, at: u64, bytes: usize) {
    for i in 0..NODES {
        let next = (i + 1) % NODES * NODE;
        let link = if bytes == 2 { next } else { slot + next };
        let be = link.to_be_bytes();
        let range =
            ByteRange::new(GuestAddr::new(slot + i * NODE + at), bytes as u64).expect("range");
        mem.apply_commit(range, &be[8 - bytes..]).expect("mapped");
    }
}

/// CellGov's core cycles an iteration for loop `l` with its slot at `slot`.
fn cellgov(l: usize, slot: u64) -> f64 {
    let (load, off) = PROBE[l];
    let (words, head) = program(load);
    let mut mem = GuestMemory::new((slot + 0x1_0000) as usize);
    for (i, w) in words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(CODE + 4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    link_ring(&mut mem, slot, u64::from(off), load.bytes());
    let count = 2 * ITERATIONS;
    let head = CODE + 4 * head as u64;
    let exit = CODE + 4 * words.len() as u64;
    let mut rt = Runtime::new(mem, Budget::new(1), usize::MAX);
    let unit = rt.register_unit_with(|id| {
        let mut ppu = PpuExecutionUnit::new(id);
        let state = ppu.state_mut();
        state.pc = CODE;
        state.set_gpr(3, slot);
        state.set_gpr(4, count);
        state.set_gpr(5, u64::from(off));
        ppu
    });
    let state = |rt: &Runtime| {
        rt.registry()
            .get(unit)
            .and_then(|u| u.as_any().downcast_ref::<PpuExecutionUnit>())
            .expect("the PPU unit")
            .state()
            .clone()
    };
    let mut start = None;
    let bound = 4 * (words.len() as u64 + 2) * count;
    for _ in 0..bound {
        let s = state(&rt);
        if s.pc == exit {
            break;
        }
        if start.is_none() && s.pc == head && s.ctr() == ITERATIONS {
            start = Some(rt.registry().clock(unit).raw());
        }
        assert_ne!(
            rt.registry().effective_status(unit),
            Some(UnitStatus::Faulted),
            "loop {l} faults at pc {:#x}",
            s.pc
        );
        let mut step = rt
            .step()
            .unwrap_or_else(|e| panic!("loop {l} at pc {:#x}: {e:?}", s.pc));
        rt.commit_step(&mut step).expect("the step commits");
    }
    assert_eq!(state(&rt).pc, exit, "loop {l} ran to its exit");
    let start = start.expect("the loop reached its timed half");
    (rt.registry().clock(unit).raw() - start) as f64 / ITERATIONS as f64
}

/// Each loop's (CellGov, console) cycles an iteration.
fn rows() -> Vec<(f64, Console)> {
    console()
        .into_iter()
        .enumerate()
        .map(|(l, c)| (cellgov(l, slot_at(c.slot)), c))
        .collect()
}

/// The loops the model times outside the console's spread, each with the
/// cycles an iteration it may stay apart by.
///
/// - Loops 12, 13 and 15: an `lhzx` inside one sector whose result an
///   `add` uses to form the next address. The console runs the chase in 8
///   cycles an iteration, as it runs an `lwzx` chase. The model gives the
///   load's result to the `add` and the `add`'s result to the next load
///   2 cycles later than that.
const RESIDUALS: [(usize, f64); 3] = [(12, 2.5), (13, 2.5), (15, 2.5)];

#[test]
fn the_issue_model_times_each_sector_cross_loop_within_the_console_s_spread() {
    let rows = rows();
    for (l, (cellgov, c)) in rows.iter().enumerate() {
        let apart = (cellgov - c.fastest).abs();
        let within = apart <= c.slowest - c.fastest;
        match RESIDUALS.iter().find(|r| r.0 == l) {
            Some(&(_, bound)) => {
                assert!(
                    !within,
                    "loop {l} now lands within the console's spread; drop it from the \
                     residuals: {rows:?}",
                );
                assert!(
                    apart < bound,
                    "loop {l}: CellGov {cellgov:.2} cycles an iteration, the console's \
                     fastest {:.2}, more than {bound} apart: {rows:?}",
                    c.fastest,
                );
            }
            None => assert!(
                within,
                "loop {l} ({:?} at {}): CellGov {cellgov:.2} cycles an iteration, the \
                 console's {:.2} to {:.2}: {rows:?}",
                PROBE[l].0, PROBE[l].1, c.fastest, c.slowest,
            ),
        }
    }
}

/// Each loop whose load crosses a sector, with the loop of the same load
/// at an offset in one sector.
const CROSS_PAIRS: [(usize, usize); 9] = [
    (2, 0),
    (3, 0),
    (4, 0),
    (5, 0),
    (6, 0),
    (9, 7),
    (10, 7),
    (11, 7),
    (14, 12),
];

/// Each loop whose load ends at its sector's end or is misaligned inside
/// one sector, with the aligned loop of the same load.
const IN_SECTOR_PAIRS: [(usize, usize); 4] = [(1, 0), (8, 7), (13, 12), (15, 12)];

#[test]
fn a_load_pays_the_split_only_when_its_bytes_cross_a_sector() {
    let rows = rows();
    for (who, cycles) in [
        ("CellGov", rows.iter().map(|r| r.0).collect::<Vec<_>>()),
        ("console", rows.iter().map(|r| r.1.fastest).collect()),
    ] {
        for (inside, aligned) in IN_SECTOR_PAIRS {
            let spread = rows[aligned].1.slowest - rows[aligned].1.fastest;
            assert!(
                (cycles[inside] - cycles[aligned]).abs() <= spread,
                "{who}: loop {inside} at {:.2} cycles, aligned loop {aligned} at {:.2}",
                cycles[inside],
                cycles[aligned],
            );
        }
        for (across, aligned) in CROSS_PAIRS {
            assert!(
                cycles[across] > 4.0 * cycles[aligned],
                "{who}: loop {across} at {:.2} cycles pays the split over loop {aligned} \
                 at {:.2}",
                cycles[across],
                cycles[aligned],
            );
        }
    }
}

/// The `ldx` loop whose word halves each stay in one sector.
const WORD_SPLIT: usize = 2;

/// The `ldx` loops where a word half crosses the sector too.
const BYTE_RETRY: [usize; 4] = [3, 4, 5, 6];

#[test]
fn a_doubleword_whose_word_half_still_crosses_pays_the_byte_retry() {
    let rows = rows();
    for (who, cycles) in [
        ("CellGov", rows.iter().map(|r| r.0).collect::<Vec<_>>()),
        ("console", rows.iter().map(|r| r.1.fastest).collect()),
    ] {
        for retry in BYTE_RETRY {
            assert!(
                cycles[retry] > cycles[WORD_SPLIT] + 10.0,
                "{who}: loop {retry} at {:.2} cycles, the word split of loop {WORD_SPLIT} \
                 at {:.2}",
                cycles[retry],
                cycles[WORD_SPLIT],
            );
        }
    }
}
