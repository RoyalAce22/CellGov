//! CellGov's PPU timing against the `ppu_issue_viq_depth` console
//! capture: sixteen independent `addi` timed beside a sibling whose every
//! round runs N dependent `fdiv` and then 256 independent `addi`.
//!
//! The replays run the probe's threads through the runtime at the
//! runtime's own step.

use cellgov_core::{default_budget_for_mode, Runtime, RuntimeMode};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_issue::{PPU_CYCLES_PER_TB_TICK, PPU_VIQ_HELD_OPS};

const CONSOLE: &str = "../../tests/micro/ppu_issue_viq_depth/ps3/cech20-cex-493/observation.json";

/// The payload's word 510 when the probe wrote every word.
const DONE: u64 = 0x5050_5644;

/// The runs that count in each phase.
const REPS: usize = 15;

/// The sibling's dependent `fdiv` a round in phases 1 to 12; phase 0 has
/// no sibling.
const DIVIDES: [usize; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 16];

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 4088);
    let w: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[510], DONE, "the probe wrote every word");
    assert_eq!(w[508], REPS as u64, "fifteen runs that count a phase");
    for p in 0..=DIVIDES.len() {
        let base = 390 + 9 * p;
        assert_eq!(w[base], REPS as u64, "phase {p} timed fifteen runs");
        if p > 0 {
            assert_eq!(w[base + 4], 0, "phase {p}'s sibling was created");
            assert_eq!(w[base + 5], 0, "phase {p}'s sibling joined");
            assert_eq!(w[base + 6], SIBLING_EXIT, "phase {p}'s sibling exited");
            assert_eq!(w[base + 7], 1, "phase {p}'s sibling ran");
            assert_eq!(w[base + 8], 0, "phase {p}'s sleep returned");
        }
    }
    w
}

/// The timed loop's iterations a run.
fn iterations(w: &[u64]) -> u64 {
    w[509]
}

/// Phase `p`'s runs that count, in cycles an iteration of the timed loop.
fn console_runs(w: &[u64], p: usize) -> Vec<f64> {
    (0..REPS)
        .map(|r| (w[2 * (REPS * p + r)] * PPU_CYCLES_PER_TB_TICK) as f64 / iterations(w) as f64)
        .collect()
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[s.len() / 2]
}

// The probe's instruction words.

/// `addi r17..r20,1` in turn.
const ADDI: [u32; 4] = [0x3a31_0001, 0x3a52_0001, 0x3a73_0001, 0x3a94_0001];
/// `fdiv f0,f0,f12`.
const FDIV: u32 = 0xfc00_6024;
/// `li r14,4`; `mtctr r14`.
const LI_R14_4: u32 = 0x39c0_0004;
const MTCTR_R14: u32 = 0x7dc9_03a6;
/// `lwz r0,0(r10)`, the stop flag; `cmpwi r0,0`.
const LWZ_STOP: u32 = 0x800a_0000;
const CMPWI_R0_0: u32 = 0x2c00_0000;
/// `ld r9,0(r8)`; `addi r9,r9,1`; `std r9,0(r8)`: the round count.
const COUNT_ROUND: [u32; 3] = [0xe928_0000, 0x3929_0001, 0xf928_0000];

/// `mftb rt`.
const fn mftb(rt: u32) -> u32 {
    0x7c0c_42e6 | rt << 21
}

/// `b` from word `from` to word `to`.
fn b(from: usize, to: usize) -> u32 {
    0x4800_0000 | ((4 * (to as i64 - from as i64)) as u32 & 0x03ff_fffc)
}

/// `bne cr0` from word `from` to word `to`.
fn bne(from: usize, to: usize) -> u32 {
    0x4082_0000 | ((4 * (to as i64 - from as i64)) as u32 & 0xfffc)
}

/// `bdnz` from word `from` to word `to`.
fn bdnz(from: usize, to: usize) -> u32 {
    0x4200_0000 | ((4 * (to as i64 - from as i64)) as u32 & 0xfffc)
}

/// `addi r21..r24,1` in turn: the `addi` after each spaced `fdiv`.
const SPACE: [u32; 4] = [0x3ab5_0001, 0x3ad6_0001, 0x3af7_0001, 0x3b18_0001];

/// The sibling's words.
///
/// Each round runs:
/// 1. `n` dependent `fdiv`, `k` `addi` after each;
/// 2. four passes of 64 `addi`;
/// 3. the stop flag's test;
/// 4. the round count and the branch back.
///
/// The exit spins.
fn sibling(n: usize, k: usize) -> Vec<u32> {
    let mut w = Vec::new();
    for _ in 0..n {
        w.push(FDIV);
        w.extend(SPACE.iter().cycle().take(k));
    }
    w.push(LI_R14_4);
    w.push(MTCTR_R14);
    let top = w.len();
    w.extend((0..64).map(|i| ADDI[i % 4]));
    w.push(bdnz(w.len(), top));
    w.push(LWZ_STOP);
    w.push(CMPWI_R0_0);
    // The exit is the spin after the bne, the round count and the branch back.
    let exit = w.len() + 5;
    w.push(bne(w.len(), exit));
    w.extend(COUNT_ROUND);
    w.push(b(w.len(), 0));
    w.push(b(w.len(), w.len()));
    w
}

/// Where the replay places the main thread's code, the timed loop, the
/// sibling's code, the stop flag and the round count. The loop head and
/// the sibling's head sit at a 128-byte line, as in the probe.
const MAIN_CODE: u64 = 0x1000;
const LOOP: u64 = 0x1080;
const SIBLING_CODE: u64 = 0x8000;
const STOP: u64 = 0x6000;
const COUNT: u64 = 0x6080;

/// `addic. r3,r3,-1`.
const COUNT_DOWN: u32 = 0x3463_ffff;

/// The warm-up spin's passes: the sibling runs this long before the timed
/// loop starts, as it does while the probe's main thread sleeps.
const WARM_UP: u64 = 20_000;

/// The main thread's words from `MAIN_CODE`, and the spin's address.
///
/// The thread runs:
/// 1. a warm-up spin;
/// 2. a time-base read;
/// 3. the timed loop, then a read;
/// 4. a spin.
fn main_thread() -> (Vec<u32>, u64) {
    let word = |at: u64| ((at - MAIN_CODE) / 4) as usize;
    let mut w = vec![COUNT_DOWN, bne(1, 0), mftb(25)];
    w.push(b(w.len(), word(LOOP)));
    w.resize(word(LOOP), 0x6000_0000);
    let top = w.len();
    w.extend((0..16).map(|i| ADDI[i % 4]));
    w.push(bdnz(w.len(), top));
    w.push(mftb(26));
    let spin = MAIN_CODE + 4 * w.len() as u64;
    w.push(b(w.len(), w.len()));
    (w, spin)
}

fn load_words(mem: &mut GuestMemory, at: u64, words: &[u32]) {
    for (i, w) in words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(at + 4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
}

/// 1.5 and 1.0 as doubles.
const ONE_HALF: u64 = 0x3ff8_0000_0000_0000;
const ONE: u64 = 0x3ff0_0000_0000_0000;

/// The steps a replay may take before it fails.
const STEP_LIMIT: usize = 5_000_000;

/// The timed loop beside a sibling of `n` dependent `fdiv` a round, `k`
/// `addi` after each, at the runtime's step: cycles an iteration.
pub(crate) fn replay(n: usize, k: usize, iterations: u64) -> f64 {
    let (main, spin) = main_thread();
    let mut mem = GuestMemory::new(0x1_0000);
    load_words(&mut mem, MAIN_CODE, &main);
    load_words(&mut mem, SIBLING_CODE, &sibling(n, k));
    let mut rt = Runtime::new(
        mem,
        default_budget_for_mode(RuntimeMode::FaultDriven),
        usize::MAX,
    );
    let unit = rt.register_unit_with(|id| {
        let mut ppu = PpuExecutionUnit::new(id);
        let state = ppu.state_mut();
        state.pc = MAIN_CODE;
        state.set_gpr(3, WARM_UP);
        state.set_ctr(iterations);
        ppu
    });
    rt.register_unit_with(|id| {
        let mut ppu = PpuExecutionUnit::new(id);
        let state = ppu.state_mut();
        state.pc = SIBLING_CODE;
        state.set_gpr(8, COUNT);
        state.set_gpr(10, STOP);
        state.set_fpr(0, ONE_HALF);
        state.set_fpr(12, ONE);
        ppu
    });
    let state = |rt: &Runtime| -> cellgov_ppu::state::PpuState {
        rt.registry()
            .get(unit)
            .and_then(|u| u.as_any().downcast_ref::<PpuExecutionUnit>())
            .expect("the PPU unit")
            .state()
            .clone()
    };
    for _ in 0..STEP_LIMIT {
        if state(&rt).pc == spin {
            break;
        }
        let mut step = rt.step().unwrap_or_else(|e| panic!("{n} divides: {e:?}"));
        rt.commit_step(&mut step).expect("the step commits");
    }
    let s = state(&rt);
    assert_eq!(s.pc, spin, "{n} divides: the loop ran to its end");
    // A sibling that stopped would leave the loop alone, and the replay
    // would time a loop with no sibling.
    let count = ByteRange::new(GuestAddr::new(COUNT), 8).expect("range");
    let rounds = rt.memory().read(count).expect("the round count");
    let rounds = u64::from_be_bytes(rounds[..].try_into().expect("8 bytes"));
    assert!(rounds > 0, "{n} divides: the sibling counted no round");
    ((s.gpr[26] - s.gpr[25]) * PPU_CYCLES_PER_TB_TICK) as f64 / iterations as f64
}

/// The rounds a lone replay times.
const LONE_ROUNDS: u64 = 64;

/// The sibling's round alone, as the probe measures it while the main
/// thread sleeps: cycles a round.
///
/// The replay ends from the guest. The round count starts `LONE_ROUNDS`
/// short of 2^32. The stop flag's load reads the count's high word, so the
/// round after the last counted one takes the exit.
fn replay_alone(n: usize) -> f64 {
    let mut words = vec![mftb(25)];
    words.extend(sibling(n, 0));
    let exit = words.len() - 1;
    words[exit] = mftb(26);
    words.push(b(words.len(), words.len()));
    let spin = SIBLING_CODE - 4 + 4 * (words.len() as u64 - 1);
    let mut mem = GuestMemory::new(0x1_0000);
    load_words(&mut mem, SIBLING_CODE - 4, &words);
    let count = ByteRange::new(GuestAddr::new(COUNT), 8).expect("range");
    mem.apply_commit(count, &((1u64 << 32) - LONE_ROUNDS).to_be_bytes())
        .expect("mapped");
    let mut rt = Runtime::new(
        mem,
        default_budget_for_mode(RuntimeMode::FaultDriven),
        usize::MAX,
    );
    let unit = rt.register_unit_with(|id| {
        let mut ppu = PpuExecutionUnit::new(id);
        let state = ppu.state_mut();
        state.pc = SIBLING_CODE - 4;
        state.set_gpr(8, COUNT);
        state.set_gpr(10, COUNT);
        state.set_fpr(0, ONE_HALF);
        state.set_fpr(12, ONE);
        ppu
    });
    let state = |rt: &Runtime| -> cellgov_ppu::state::PpuState {
        rt.registry()
            .get(unit)
            .and_then(|u| u.as_any().downcast_ref::<PpuExecutionUnit>())
            .expect("the PPU unit")
            .state()
            .clone()
    };
    for _ in 0..STEP_LIMIT {
        if state(&rt).pc == spin {
            break;
        }
        let mut step = rt.step().unwrap_or_else(|e| panic!("{n} divides: {e:?}"));
        rt.commit_step(&mut step).expect("the step commits");
    }
    let s = state(&rt);
    assert_eq!(s.pc, spin, "{n} divides alone: the rounds ran to the exit");
    let rounds = rt.memory().read(count).expect("the round count");
    let rounds = u64::from_be_bytes(rounds[..].try_into().expect("8 bytes"));
    assert_eq!(rounds, 1 << 32, "{n} divides alone: every round counted");
    // The rounds counted, and the last round up to its exit.
    ((s.gpr[26] - s.gpr[25]) * PPU_CYCLES_PER_TB_TICK) as f64 / (LONE_ROUNDS + 1) as f64
}

/// The console's sibling of `n` divides a round alone, over the main
/// thread's sleep before the phase: cycles a round.
fn console_alone(w: &[u64], n: usize) -> f64 {
    let phase = 1 + DIVIDES.iter().position(|&d| d == n).expect("a timed N");
    let base = 390 + 9 * phase;
    (w[base + 3] * PPU_CYCLES_PER_TB_TICK) as f64 / w[base + 2] as f64
}

#[test]
fn alone_the_sibling_of_each_n_divides_a_round_runs_near_the_console() {
    use super::console_ppu_issue_sibling_rate::TOLERANCE;
    let w = console_words();
    for &n in &DIVIDES {
        let console = console_alone(&w, n);
        let cellgov = replay_alone(n);
        assert!(
            (cellgov - console).abs() / console <= TOLERANCE,
            "{n} divides alone: CellGov {cellgov:.1} cycles a round, the console {console:.1}"
        );
    }
}

/// The fewest divides a round that fill the VSU issue queue: one runs and
/// `PPU_VIQ_HELD_OPS` wait.
const QUEUE_FULL: usize = PPU_VIQ_HELD_OPS + 1;

#[test]
fn alone_the_divides_past_a_full_queue_add_to_the_round_what_they_add_on_the_console() {
    // The round's `addi` issue while the divides wait in the queue, so the
    // divides add to the round only what outlasts the `addi`.
    let w = console_words();
    let (console_none, cellgov_none) = (console_alone(&w, 0), replay_alone(0));
    let full: Vec<usize> = DIVIDES.into_iter().filter(|&n| n >= QUEUE_FULL).collect();
    assert_eq!(full, [7, 8, 10, 12, 16]);
    for n in full {
        let console = console_alone(&w, n) - console_none;
        let cellgov = replay_alone(n) - cellgov_none;
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "{n} divides alone: CellGov adds {cellgov:.1} cycles a round, the console {console:.1}"
        );
    }
}

/// The console's median run beside a sibling of `n` divides a round.
fn console_median(w: &[u64], n: usize) -> f64 {
    let phase = 1 + DIVIDES.iter().position(|&d| d == n).expect("a timed N");
    median(&console_runs(w, phase))
}

#[test]
fn on_the_console_the_loop_slows_only_past_seven_divides_a_round() {
    // One divide runs and six wait in the VSU issue queue.
    let w = console_words();
    let one = console_median(&w, 1);
    for n in 2..=QUEUE_FULL {
        let held = console_median(&w, n);
        assert!(
            (held - one).abs() / one < 0.01,
            "{n} divides: {held:.2} cycles an iteration, one divide {one:.2}"
        );
    }
    let past = console_median(&w, QUEUE_FULL + 1);
    assert!(
        past > 1.1 * console_median(&w, QUEUE_FULL),
        "{} divides: {past:.2} cycles an iteration",
        QUEUE_FULL + 1
    );
}

/// The share of the console's value the model may lie from it.
pub(crate) const BAND: f64 = 0.05;

#[test]
fn beside_a_sibling_of_one_to_sixteen_divides_a_round_the_model_times_the_loop_near_the_console() {
    // This capture's run beside no divides did not repeat: two later runs
    // time that loop at the one-divide rate. The `ppu_issue_viq_layout`
    // capture holds the model to it.
    let w = console_words();
    let iterations = iterations(&w);
    for &n in DIVIDES.iter().filter(|&&n| n > 0) {
        let console = console_median(&w, n);
        let cellgov = replay(n, 0, iterations);
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "{n} divides: CellGov {cellgov:.2} cycles an iteration, the console {console:.2}"
        );
    }
}
