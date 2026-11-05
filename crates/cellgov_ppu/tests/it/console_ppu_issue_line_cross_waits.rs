//! CellGov's PPU timing against the `ppu_issue_line_cross_waits` console
//! capture: what an access whose bytes cross from one 128-byte line into
//! the next leaves in, or waits for in, its second line.

use cellgov_core::Runtime;
use cellgov_exec::UnitStatus;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_issue::{PPU_CACHE_LINE_BYTES, PPU_CYCLES_PER_TB_TICK};
use cellgov_time::Budget;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_line_cross_waits/ps3/cech20-cex-493/observation.json";

/// The payload's word 58 when the probe wrote every word.
const DONE: u64 = 0x5050_4c57;

/// The probe's loops.
const LOOPS: usize = 14;

/// The bytes of one node: two lines.
const NODE: u64 = 2 * PPU_CACHE_LINE_BYTES;

/// The nodes a cold loop chases once.
const COLD_NODES: u64 = 96;

/// The nodes of a flush loop's ring.
const WARM_NODES: u64 = 64;

/// The nodes of the touch loops' 8 MB ring.
const RING_NODES: u64 = 8 * 1024 * 1024 / NODE;

/// What the probe runs on each node of a cold loop after it flushes both
/// lines of the node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    /// `stmw r28,off(node)`.
    Stmw(u16),
    /// `lswx r28,node,r8` with r8 and the XER byte count 0.
    LswxZero,
    /// `lbz r7,0(node)`.
    Lbz,
    /// Nothing.
    Nothing,
}

/// A loop's shape.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Flush both lines of each of 96 nodes, run a step on each, then
    /// chase them once.
    Cold(Step),
    /// A 64-node ring chased with a `dcbf` of a line of each node.
    Flush,
    /// The 8 MB ring chased with two `dcbt`s eight nodes ahead.
    Touch,
}

/// `addi r10,r9,a; dcbf 0,r10`, which flushes line A (`a` 0) or B (`a` 128).
fn flush(a: u32) -> Vec<u32> {
    vec![0x3949_0000 | a, 0x7c00_50ac]
}

/// `addi r10,r9,a; dcbt 0,r10; addi r11,r9,b; dcbt 0,r11`.
fn touch(a: u32, b: u32) -> Vec<u32> {
    vec![0x3949_0000 | a, 0x7c00_522c, 0x3969_0000 | b, 0x7c00_5a2c]
}

/// Line A of the node eight ahead.
const AHEAD_A: u32 = 2048;
/// Line B of the node eight ahead.
const AHEAD_B: u32 = 2048 + 128;

/// Each loop's shape, the words before its chase load, and the chase
/// offset, in the probe's order.
fn probe() -> [(Kind, Vec<u32>, u16); LOOPS] {
    [
        (Kind::Cold(Step::Stmw(96)), vec![], 192),
        (Kind::Cold(Step::Stmw(120)), vec![], 192),
        (Kind::Cold(Step::Stmw(128)), vec![], 192),
        (Kind::Flush, flush(128), 124),
        (Kind::Flush, flush(0), 124),
        (Kind::Flush, flush(128), 112),
        (Kind::Flush, flush(0), 112),
        (Kind::Touch, touch(AHEAD_A, AHEAD_B), 124),
        (Kind::Touch, touch(AHEAD_A, AHEAD_A), 124),
        (Kind::Touch, touch(AHEAD_B, AHEAD_B), 124),
        (Kind::Touch, touch(AHEAD_A, AHEAD_B), 112),
        (Kind::Cold(Step::LswxZero), vec![], 112),
        (Kind::Cold(Step::Lbz), vec![], 112),
        (Kind::Cold(Step::Nothing), vec![], 112),
    ]
}

/// One loop of the console's payload.
#[derive(Clone, Copy, Debug)]
struct Console {
    /// The fastest run, in core cycles an iteration.
    fastest: f64,
    /// The effective address of the loop's slot or ring.
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
                slot: w[28 + l],
            }
        })
        .collect()
}

/// Where the replay places its code.
const CODE: u64 = 0x1000;

/// Where the replay places a slot or ring: the console's at the same place
/// in a 64 KB span, which keeps its L1 and L2 sets.
fn slot_at(kind: Kind, console_slot: u64) -> u64 {
    let base = if kind == Kind::Touch {
        0x100_0000
    } else {
        0x10_0000
    };
    base + console_slot % 0x1_0000
}

/// Iterations a flush or touch replay times. It runs as many before them,
/// so the timed half starts with the caches and touches in their steady
/// state.
const WARM_ITERATIONS: u64 = 512;

fn bdnz_to(from: usize, to: usize) -> u32 {
    let disp = (to as i64 - from as i64) * 4;
    0x4200_0000 | (disp as u32 & 0xfffc)
}

/// The probe's preparation for a cold loop, with r3 the slot: flush both
/// lines of each node, `sync`, run `step` on each node, `sync`.
fn prepare(step: Step) -> Vec<u32> {
    let nodes = 0x3920_0000 | COLD_NODES as u32; // li r9,COLD_NODES
    let mut w = vec![nodes, 0x7d29_03a6, 0x3923_0080]; // mtctr r9; addi r9,r3,128
    let head = w.len();
    w.extend([0x3909_ff80, 0x7c00_48ac, 0x7c00_40ac]); // addi r8,r9,-128; dcbf 0,r9; dcbf 0,r8
    w.push(0x3929_0100); // addi r9,r9,256
    w.push(bdnz_to(w.len(), head));
    w.push(0x7c00_04ac); // sync
                         // mtctr r9; mr r10,r3; li r8,0; mtxer r8
    w.extend([nodes, 0x7d29_03a6, 0x7c6a_1b78, 0x3900_0000, 0x7d01_03a6]);
    let head = w.len();
    match step {
        Step::Stmw(off) => w.push(0xbf8a_0000 | u32::from(off)),
        Step::LswxZero => w.push(0x7f8a_442a),
        Step::Lbz => w.push(0x88ea_0000),
        Step::Nothing => {}
    }
    w.push(0x394a_0100); // addi r10,r10,256
    w.push(bdnz_to(w.len(), head));
    w.push(0x7c00_04ac); // sync
    w
}

/// The words of a loop, and the index of its timed loop's head.
fn program(kind: Kind, body: &[u32], off: u16) -> (Vec<u32>, usize) {
    let mut w = match kind {
        Kind::Cold(step) => prepare(step),
        Kind::Flush | Kind::Touch => Vec::new(),
    };
    w.push(0x7c69_1b78); // mr r9,r3
    w.push(0x7c89_03a6); // mtctr r4
    let head = w.len();
    w.extend_from_slice(body);
    w.push(0xe929_0000 | u32::from(off)); // ld r9,off(r9)
    w.push(bdnz_to(w.len(), head));
    (w, head)
}

/// Writes the ring of `nodes` nodes at `slot`: each node holds the next
/// node's address as a doubleword at `at`.
fn link_ring(mem: &mut GuestMemory, slot: u64, nodes: u64, at: u64) {
    for i in 0..nodes {
        let next = slot + (i + 1) % nodes * NODE;
        let range = ByteRange::new(GuestAddr::new(slot + i * NODE + at), 8).expect("range");
        mem.apply_commit(range, &next.to_be_bytes())
            .expect("mapped");
    }
}

/// CellGov's core cycles an iteration for loop `l` with its slot at
/// `slot`.
fn cellgov(l: usize, kind: Kind, body: &[u32], off: u16, slot: u64) -> f64 {
    let (words, head) = program(kind, body, off);
    let nodes = match kind {
        Kind::Cold(_) => COLD_NODES,
        Kind::Flush => WARM_NODES,
        Kind::Touch => RING_NODES,
    };
    let mut mem = GuestMemory::new((slot + nodes * NODE + 0x1_0000) as usize);
    for (i, w) in words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(CODE + 4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    link_ring(&mut mem, slot, nodes, u64::from(off));
    let (count, timed) = match kind {
        Kind::Cold(_) => (COLD_NODES, COLD_NODES),
        Kind::Flush | Kind::Touch => (2 * WARM_ITERATIONS, WARM_ITERATIONS),
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
        .zip(probe())
        .enumerate()
        .map(|(l, (c, (kind, body, off)))| (cellgov(l, kind, &body, off, slot_at(kind, c.slot)), c))
        .collect()
}

/// CellGov's and the console's fastest cycles an iteration for each loop.
fn sides() -> [(&'static str, Vec<f64>); 2] {
    let rows = rows();
    [
        ("CellGov", rows.iter().map(|r| r.0).collect()),
        ("console", rows.iter().map(|r| r.1.fastest).collect()),
    ]
}

/// Store loops: the `stmw` writes line A only, runs 8 bytes into line B,
/// or writes line B only; then the chase reads line B.
const STORE_A_ONLY: usize = 0;
const STORE_ACROSS: usize = 1;
const STORE_B_ONLY: usize = 2;

#[test]
fn a_store_that_runs_into_a_flushed_second_line_brings_that_line_back() {
    for (who, cycles) in sides() {
        let memory = cycles[STORE_A_ONLY];
        for l in [STORE_ACROSS, STORE_B_ONLY] {
            assert!(
                cycles[l] < memory / 4.0,
                "{who}: loop {l} at {:.2} cycles finds line B nearer than memory, a trip \
                 of {memory:.2}",
                cycles[l],
            );
        }
    }
}

/// Flush loops: `dcbf` of line B or line A, then a load across both, or a
/// load in line A alone.
const FLUSH_B_ACROSS: usize = 3;
const FLUSH_A_ACROSS: usize = 4;
const FLUSH_B_IN_A: usize = 5;
const FLUSH_A_IN_A: usize = 6;

#[test]
fn a_load_into_a_second_line_just_flushed_waits_as_for_its_first_line() {
    for (who, cycles) in sides() {
        let first = cycles[FLUSH_A_ACROSS];
        assert!(
            (cycles[FLUSH_B_ACROSS] - first).abs() < first / 20.0,
            "{who}: loop {FLUSH_B_ACROSS} at {:.2} cycles waits as loop {FLUSH_A_ACROSS} \
             does at {first:.2}",
            cycles[FLUSH_B_ACROSS],
        );
        assert!(
            cycles[FLUSH_B_IN_A] < cycles[FLUSH_A_IN_A] / 4.0,
            "{who}: loop {FLUSH_B_IN_A} at {:.2} cycles never reads the flushed line; loop \
             {FLUSH_A_IN_A} at {:.2} does",
            cycles[FLUSH_B_IN_A],
            cycles[FLUSH_A_IN_A],
        );
    }
}

/// Touch loops: `dcbt` of line A and line B eight nodes ahead, of line A
/// alone, or of line B alone, then a load across both; and both touches
/// with a load in line A alone.
const TOUCH_BOTH_ACROSS: usize = 7;
const TOUCH_A_ACROSS: usize = 8;
const TOUCH_B_ACROSS: usize = 9;
const TOUCH_BOTH_IN_A: usize = 10;

#[test]
fn a_load_across_two_lines_gains_from_a_touch_only_when_both_lines_are_touched() {
    for (who, cycles) in sides() {
        let both = cycles[TOUCH_BOTH_ACROSS];
        let one_line = cycles[TOUCH_BOTH_IN_A];
        assert!(
            (both - one_line).abs() < one_line / 10.0,
            "{who}: loop {TOUCH_BOTH_ACROSS} at {both:.2} cycles costs what loop \
             {TOUCH_BOTH_IN_A} does at {one_line:.2}",
        );
        for l in [TOUCH_A_ACROSS, TOUCH_B_ACROSS] {
            assert!(
                cycles[l] > 2.0 * both,
                "{who}: loop {l} at {:.2} cycles waits for its untouched line; loop \
                 {TOUCH_BOTH_ACROSS} at {both:.2} does not",
                cycles[l],
            );
        }
    }
}

/// Zero-count loops: a count-0 `lswx`, an `lbz` or nothing at each
/// flushed node, then a chase of line A.
const LSWX_ZERO: usize = 11;
const LBZ: usize = 12;
const UNTOUCHED: usize = 13;

#[test]
fn a_count_zero_lswx_brings_the_line_at_its_address_into_the_l1() {
    for (who, cycles) in sides() {
        let memory = cycles[UNTOUCHED];
        assert!(
            cycles[LBZ] < memory / 4.0,
            "{who}: loop {LBZ} at {:.2} cycles finds its lines in the L1",
            cycles[LBZ],
        );
        assert!(
            cycles[LSWX_ZERO] < memory / 4.0,
            "{who}: loop {LSWX_ZERO} at {:.2} cycles finds the lines a count-0 lswx \
             touched nearer than memory, a trip of {memory:.2}",
            cycles[LSWX_ZERO],
        );
    }
}
