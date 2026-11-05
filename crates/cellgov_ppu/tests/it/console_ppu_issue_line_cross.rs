//! CellGov's PPU timing against the `ppu_issue_line_cross` console
//! capture: loads and load and store multiples whose bytes cross from one
//! 128-byte line into the next.

use cellgov_core::Runtime;
use cellgov_exec::UnitStatus;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_issue::{PPU_CACHE_LINE_BYTES, PPU_CYCLES_PER_TB_TICK};
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/ppu_issue_line_cross/ps3/cech20-cex-493/observation.json";

/// The payload's word 58 when the probe wrote every word.
const DONE: u64 = 0x5050_4c58;

/// The probe's loops.
const LOOPS: usize = 14;

/// The bytes of one node: two lines.
const NODE: u64 = 2 * PPU_CACHE_LINE_BYTES;

/// The nodes a warm loop's ring holds.
const WARM_NODES: u64 = 64;

/// The nodes a cold loop chases once.
const COLD_NODES: u64 = 96;

/// What the probe does before each run of a loop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Nothing: earlier runs leave the loop's lines in the L1.
    Warm,
    /// Flush each node's second line and load each first line.
    SecondCold,
    /// Flush both lines of each node.
    BothCold,
    /// Nothing; the loop stores to one fixed node.
    Store,
}

/// The access in a loop's body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Access {
    /// `ld r9,off(r9)`.
    Ld,
    /// `lmw r28,off(r9); mr r9,r31`.
    Lmw,
    /// `stmw r28,off(r3)`.
    Stmw,
}

/// Each loop's access, offset into the node and kind, in the probe's
/// order.
const PROBE: [(Access, u16, Kind); LOOPS] = [
    (Access::Ld, 112, Kind::Warm),
    (Access::Ld, 116, Kind::Warm),
    (Access::Ld, 92, Kind::Warm),
    (Access::Ld, 124, Kind::Warm),
    (Access::Ld, 112, Kind::SecondCold),
    (Access::Ld, 124, Kind::SecondCold),
    (Access::Ld, 112, Kind::BothCold),
    (Access::Ld, 124, Kind::BothCold),
    (Access::Lmw, 96, Kind::Warm),
    (Access::Lmw, 120, Kind::Warm),
    (Access::Lmw, 96, Kind::SecondCold),
    (Access::Lmw, 120, Kind::SecondCold),
    (Access::Stmw, 96, Kind::Store),
    (Access::Stmw, 120, Kind::Store),
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
    assert_eq!(p.len(), 472);
    let w: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[58], DONE, "the probe wrote every word");
    (0..LOOPS)
        .map(|l| {
            let n = w[42 + l] as f64;
            let per = |ticks: u64| (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / n;
            Console {
                fastest: per(w[2 * l]),
                slowest: per(w[2 * l + 1]),
                slot: w[28 + l],
            }
        })
        .collect()
}

/// Where the replay places its code.
const CODE: u64 = 0x1000;

/// Where the replay places a slot: the console's slot at the same place
/// in a 64 KB span, which keeps its L1 and L2 sets.
fn slot_at(console_slot: u64) -> u64 {
    0x10_0000 + console_slot % 0x1_0000
}

/// Iterations a warm or store replay times. It runs as many before them,
/// which leave the loop's lines in the caches as the probe's earlier runs
/// do.
const WARM_ITERATIONS: u64 = 512;

fn bdnz_to(from: usize, to: usize) -> u32 {
    let disp = (to as i64 - from as i64) * 4;
    0x4200_0000 | (disp as u32 & 0xfffc)
}

/// The probe's preparation for a cold loop, with r3 the slot.
///
/// 1. Flush the second line of each node, and for `BothCold` its first
///    line, then `sync`.
/// 2. For `SecondCold`, load a byte of each first line, then `sync`.
fn prepare(kind: Kind) -> Vec<u32> {
    let nodes = 0x3920_0000 | COLD_NODES as u32; // li r9,COLD_NODES
    let mut w = vec![nodes, 0x7d29_03a6, 0x3923_0080]; // mtctr r9; addi r9,r3,128
    let head = w.len();
    w.push(0x3909_ff80); // addi r8,r9,-128
    w.push(0x7c00_48ac); // dcbf 0,r9
    if kind == Kind::BothCold {
        w.push(0x7c00_40ac); // dcbf 0,r8
    }
    w.push(0x3929_0100); // addi r9,r9,256
    w.push(bdnz_to(w.len(), head));
    w.push(0x7c00_04ac); // sync
    if kind == Kind::SecondCold {
        w.extend([nodes, 0x7d29_03a6, 0x7c6a_1b78]); // mtctr r9; mr r10,r3
        let head = w.len();
        w.push(0x890a_0000); // lbz r8,0(r10)
        w.push(0x394a_0100); // addi r10,r10,256
        w.push(bdnz_to(w.len(), head));
        w.push(0x7c00_04ac); // sync
    }
    w
}

/// The words of a loop, and the index of its timed loop's head.
fn program(access: Access, off: u16, kind: Kind) -> (Vec<u32>, usize) {
    let mut w = match kind {
        Kind::SecondCold | Kind::BothCold => prepare(kind),
        Kind::Warm | Kind::Store => Vec::new(),
    };
    w.push(0x7c69_1b78); // mr r9,r3
    w.push(0x7c89_03a6); // mtctr r4
    let head = w.len();
    let off = u32::from(off);
    match access {
        Access::Ld => w.push(0xe929_0000 | off),
        Access::Lmw => w.extend([0xbb89_0000 | off, 0x7fe9_fb78]),
        Access::Stmw => w.push(0xbf83_0000 | off),
    }
    w.push(bdnz_to(w.len(), head));
    (w, head)
}

/// Writes the ring of `nodes` nodes at `slot`: each node holds the next
/// node's address in `bytes` bytes at `at`.
fn link_ring(mem: &mut GuestMemory, slot: u64, nodes: u64, at: u64, bytes: usize) {
    for i in 0..nodes {
        let next = slot + (i + 1) % nodes * NODE;
        let be = next.to_be_bytes();
        let range =
            ByteRange::new(GuestAddr::new(slot + i * NODE + at), bytes as u64).expect("range");
        mem.apply_commit(range, &be[8 - bytes..]).expect("mapped");
    }
}

/// CellGov's core cycles an iteration for loop `l` with its slot at
/// `slot`.
fn cellgov(l: usize, slot: u64) -> f64 {
    let (access, off, kind) = PROBE[l];
    let (words, head) = program(access, off, kind);
    let mut mem = GuestMemory::new((slot + 0x1_0000) as usize);
    for (i, w) in words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(CODE + 4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    let nodes = if kind == Kind::Warm {
        WARM_NODES
    } else {
        COLD_NODES
    };
    match access {
        Access::Ld => link_ring(&mut mem, slot, nodes, u64::from(off), 8),
        Access::Lmw => link_ring(&mut mem, slot, nodes, u64::from(off) + 12, 4),
        Access::Stmw => {}
    }
    let (count, timed) = match kind {
        Kind::Warm | Kind::Store => (2 * WARM_ITERATIONS, WARM_ITERATIONS),
        Kind::SecondCold | Kind::BothCold => (COLD_NODES, COLD_NODES),
    };
    let head = CODE + 4 * head as u64;
    let exit = CODE + 4 * words.len() as u64;
    let mut rt = Runtime::new(mem, Budget::new(1), usize::MAX);
    let unit = rt.register_unit_with(|id| {
        let mut ppu = PpuExecutionUnit::new(id);
        let state = ppu.state_mut();
        state.pc = CODE;
        state.set_gpr(3, slot);
        state.set_gpr(4, count);
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
    let bound = 4 * (words.len() as u64 + 2) * (count + 2 * COLD_NODES);
    for _ in 0..bound {
        let s = state(&rt);
        if s.pc == exit {
            break;
        }
        if start.is_none() && s.pc == head && s.ctr() == timed {
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
    (rt.registry().clock(unit).raw() - start) as f64 / timed as f64
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
/// - Loops 2, 3, 5 and 7: the console pays for an `ld` across two 32-byte
///   sectors, and CellGov times it as an aligned `ld`.
///   [CBE-Handbook p:730 s:A.1.3.2] such a load is flushed, refetched and
///   split into microcode.
/// - Loops 6 and 11: the model's trip to memory comes from a shuffled
///   chase, and this probe chases its nodes in address order.
/// - Loops 8, 10, 12 and 13: `lmw` and `stmw` take a few cycles more on
///   the console than the handbook's microword counts give, as in the
///   `ppu_issue_multiple` capture.
const RESIDUALS: [(usize, f64); 10] = [
    (2, 47.0),
    (3, 47.0),
    (5, 78.0),
    (6, 43.0),
    (7, 38.0),
    (8, 3.0),
    (10, 4.0),
    (11, 30.0),
    (12, 4.0),
    (13, 4.0),
];

#[test]
fn the_issue_model_times_each_line_cross_loop_within_the_console_s_spread() {
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
                "loop {l}: CellGov {cellgov:.2} cycles an iteration, the console's {:.2} \
                 to {:.2}: {rows:?}",
                c.fastest, c.slowest,
            ),
        }
    }
}

/// The loop that chases nodes whose lines are both flushed, one line an
/// access: one trip to memory an iteration.
const MEMORY_CHASE: usize = 6;

/// Each chase across two lines with its second line flushed, with the
/// same chase on the first line alone.
const SECOND_COLD: [(usize, usize); 2] = [(5, 4), (11, 10)];

#[test]
fn an_access_into_a_flushed_second_line_waits_for_a_trip_to_memory() {
    let rows = rows();
    for (who, cycles) in [
        ("CellGov", rows.iter().map(|r| r.0).collect::<Vec<_>>()),
        ("console", rows.iter().map(|r| r.1.fastest).collect()),
    ] {
        let memory = cycles[MEMORY_CHASE];
        for (across, one_line) in SECOND_COLD {
            assert!(
                cycles[one_line] < memory / 4.0,
                "{who}: loop {one_line} at {:.2} cycles finds its first line in the L1",
                cycles[one_line],
            );
            assert!(
                cycles[across] >= memory,
                "{who}: loop {across} at {:.2} cycles waits for its second line from \
                 memory, a trip of {memory:.2}",
                cycles[across],
            );
        }
    }
}

/// Each warm access across two lines, with the same access on one line.
/// The `ld` on one line crosses two of its 32-byte sectors, as the `ld`
/// across two lines does.
const WARM_PAIRS: [(usize, usize); 3] = [(3, 2), (9, 8), (13, 12)];

#[test]
fn an_access_into_a_second_line_in_the_l1_costs_what_it_costs_on_one_line() {
    let rows = rows();
    for (across, one_line) in WARM_PAIRS {
        let (c_across, c_one) = (rows[across].1, rows[one_line].1);
        let spread = c_one.slowest - c_one.fastest;
        assert!(
            (c_across.fastest - c_one.fastest).abs() <= spread,
            "console: loop {across} at {:.2} cycles, loop {one_line} at {:.2} to {:.2}",
            c_across.fastest,
            c_one.fastest,
            c_one.slowest,
        );
        assert!(
            (rows[across].0 - rows[one_line].0).abs() <= spread,
            "CellGov: loop {across} at {:.2} cycles, loop {one_line} at {:.2}, more than \
             the console's spread of {spread:.2} apart",
            rows[across].0,
            rows[one_line].0,
        );
    }
}
