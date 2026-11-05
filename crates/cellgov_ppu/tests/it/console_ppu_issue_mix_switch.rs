//! CellGov's PPU timing against the `ppu_issue_mix_switch` console
//! capture: sixteen independent `addi` timed in two halves, beside a
//! sibling that changes instruction class between them or within each of
//! its rounds.
//!
//! The runtime gives a PPE thread its sibling's issue pressure once a
//! step. The replays here run the probe's two threads through the
//! runtime at one instruction a step and at the runtime's own step.

use cellgov_core::{default_budget_for_mode, Runtime, RuntimeMode};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/ppu_issue_mix_switch/ps3/cech20-cex-493/observation.json";

/// The payload's word 435 when the probe wrote every word.
const DONE: u64 = 0x5050_5357;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The runs that count in each phase.
const REPS: usize = 15;

/// The phases: none, then the five siblings.
const PHASES: usize = 6;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 3488);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[435], DONE, "the probe wrote every word");
    assert_eq!(words[433], REPS as u64, "fifteen runs that count a phase");
    for p in 0..PHASES {
        assert_eq!(
            words[360 + 2 * p],
            REPS as u64,
            "phase {p} timed fifteen runs that count"
        );
    }
    for p in 1..PHASES {
        let s = 372 + 10 * p;
        assert_eq!(words[s + 4], 0, "phase {p}'s sibling was created");
        assert_eq!(words[s + 5], 0, "phase {p}'s sibling joined");
        assert_eq!(words[s + 6], SIBLING_EXIT, "phase {p}'s sibling exited");
        assert_eq!(words[s + 7], 1, "phase {p}'s sibling ran");
        assert_eq!(words[s + 8], 0, "phase {p}'s first sleep returned");
        assert_eq!(words[s + 9], 0, "phase {p}'s second sleep returned");
    }
    words
}

/// The iterations of each half.
fn half_iterations(w: &[u64]) -> u64 {
    w[434]
}

/// Half `h` of each run that counts in phase `p`, in cycles an iteration.
fn console_runs(w: &[u64], p: usize, h: usize) -> Vec<f64> {
    (0..REPS)
        .map(|r| {
            let ticks = w[4 * (REPS * p + r) + h];
            (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / half_iterations(w) as f64
        })
        .collect()
}

/// The mean of `v`.
fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

// The probe's instruction words.

/// `addi r17..r20,1` in turn.
const ADDI: [u32; 4] = [0x3a31_0001, 0x3a52_0001, 0x3a73_0001, 0x3a94_0001];
/// `fdiv f0,f0,f12`.
const FDIV: u32 = 0xfc00_6024;
/// `li r14,4`; `mtctr r14`.
const LI_R14_4: u32 = 0x39c0_0004;
const MTCTR_R14: u32 = 0x7dc9_03a6;
/// `lwz r0,0(r7)`, the flag; `lwz r0,0(r10)`, the stop flag.
const LWZ_FLAG: u32 = 0x8007_0000;
const LWZ_STOP: u32 = 0x800a_0000;
/// `cmpwi r0,0`.
const CMPWI_R0_0: u32 = 0x2c00_0000;
/// `ld r9,0(r8)`; `addi r9,r9,1`; `std r9,0(r8)`: the round count.
const COUNT_ROUND: [u32; 3] = [0xe928_0000, 0x3929_0001, 0xf928_0000];
/// `li r0,1`; `stw r0,0(r7)`: set the flag.
const SET_FLAG: [u32; 2] = [0x3800_0001, 0x9007_0000];
/// `mtctr r11`.
const MTCTR_R11: u32 = 0x7d69_03a6;

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

/// The words of `n` independent `addi`, four targets in turn.
fn addi(n: usize) -> Vec<u32> {
    (0..n).map(|i| ADDI[i % 4]).collect()
}

/// A sibling round's body: 8 dependent `fdiv` or 64 independent `addi`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Fdiv,
    Addi,
}

impl Class {
    fn words(self) -> Vec<u32> {
        match self {
            Class::Fdiv => vec![FDIV; 8],
            Class::Addi => addi(64),
        }
    }
}

/// `li r14,4; mtctr r14`, `class` to a `bdnz`.
fn four_passes(w: &mut Vec<u32>, class: Class) {
    w.push(LI_R14_4);
    w.push(MTCTR_R14);
    let top = w.len();
    w.extend(class.words());
    w.push(bdnz(w.len(), top));
}

/// The end of every round: the stop flag's test, the round count's
/// increment and the branch back to the head at word 0, then the exit.
fn tail(w: &mut Vec<u32>) {
    w.push(LWZ_STOP);
    w.push(CMPWI_R0_0);
    let exit = w.len() + 6;
    w.push(bne(w.len(), exit));
    w.extend(COUNT_ROUND);
    w.push(b(w.len(), 0));
    // The exit spins.
    w.push(b(w.len(), w.len()));
}

/// The sibling of phases 1 to 4: a round of `before` while the flag is
/// clear, of `after` once it is set.
fn switch_sibling(before: Class, after: Class) -> Vec<u32> {
    let mut w = vec![LWZ_FLAG, CMPWI_R0_0, 0];
    four_passes(&mut w, before);
    let jump = w.len();
    w.push(0);
    let second = w.len();
    w[2] = bne(2, second);
    four_passes(&mut w, after);
    w[jump] = b(jump, w.len());
    tail(&mut w);
    w
}

/// The sibling of phase 5: 8 dependent `fdiv`, then four passes of 64
/// independent `addi`, each round.
fn alternating_sibling() -> Vec<u32> {
    let mut w = Class::Fdiv.words();
    four_passes(&mut w, Class::Addi);
    tail(&mut w);
    w
}

/// Phase `p`'s sibling, or `None` in phase 0.
fn sibling(p: usize) -> Option<Vec<u32>> {
    match p {
        0 => None,
        1 => Some(switch_sibling(Class::Fdiv, Class::Fdiv)),
        2 => Some(switch_sibling(Class::Addi, Class::Addi)),
        3 => Some(switch_sibling(Class::Fdiv, Class::Addi)),
        4 => Some(switch_sibling(Class::Addi, Class::Fdiv)),
        _ => Some(alternating_sibling()),
    }
}

/// Where the replay places the main thread's code, each half's loop, the
/// sibling's code and the flags. Each loop head and the sibling's head sit
/// at a 128-byte line, as in the probe.
const MAIN_CODE: u64 = 0x1000;
const HALF_ONE: u64 = 0x1080;
const HALF_TWO: u64 = 0x1100;
const SIBLING_CODE: u64 = 0x8000;
const FLAG: u64 = 0x6100;
const STOP: u64 = 0x6000;
const COUNT: u64 = 0x6080;

/// The main thread's words from `MAIN_CODE`, and the spin's address.
///
/// The thread runs:
/// 1. a time-base read;
/// 2. the first half's loop, then a read;
/// 3. the flag set;
/// 4. the second half's loop, then a read;
/// 5. a spin.
fn main_thread() -> (Vec<u32>, u64) {
    let word = |at: u64| ((at - MAIN_CODE) / 4) as usize;
    let mut w = vec![mftb(25)];
    w.push(b(1, word(HALF_ONE)));
    w.resize(word(HALF_ONE), 0x6000_0000);
    let loop_body = |w: &mut Vec<u32>| {
        let top = w.len();
        w.extend(addi(16));
        w.push(bdnz(w.len(), top));
    };
    loop_body(&mut w);
    w.push(mftb(26));
    w.extend(SET_FLAG);
    w.push(MTCTR_R11);
    w.push(b(w.len(), word(HALF_TWO)));
    w.resize(word(HALF_TWO), 0x6000_0000);
    loop_body(&mut w);
    w.push(mftb(27));
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

/// Phase `p`'s run on the runtime at `budget` instructions a step: each
/// half's cycles an iteration.
fn replay(p: usize, iterations: u64, budget: Budget) -> [f64; 2] {
    let (main, spin) = main_thread();
    let mut mem = GuestMemory::new(0x1_0000);
    load_words(&mut mem, MAIN_CODE, &main);
    let sib = sibling(p);
    if let Some(words) = &sib {
        load_words(&mut mem, SIBLING_CODE, words);
    }
    let mut rt = Runtime::new(mem, budget, usize::MAX);
    let unit = rt.register_unit_with(|id| {
        let mut ppu = PpuExecutionUnit::new(id);
        let state = ppu.state_mut();
        state.pc = MAIN_CODE;
        state.set_ctr(iterations);
        state.set_gpr(7, FLAG);
        state.set_gpr(11, iterations);
        ppu
    });
    if sib.is_some() {
        rt.register_unit_with(|id| {
            let mut ppu = PpuExecutionUnit::new(id);
            let state = ppu.state_mut();
            state.pc = SIBLING_CODE;
            state.set_gpr(7, FLAG);
            state.set_gpr(8, COUNT);
            state.set_gpr(10, STOP);
            state.set_fpr(0, ONE_HALF);
            state.set_fpr(12, ONE);
            ppu
        });
    }
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
        let mut step = rt.step().unwrap_or_else(|e| panic!("phase {p}: {e:?}"));
        rt.commit_step(&mut step).expect("the step commits");
    }
    let s = state(&rt);
    assert_eq!(s.pc, spin, "phase {p} ran to its end");
    let cycles = |a: u64, z: u64| ((z - a) * PPU_CYCLES_PER_TB_TICK) as f64 / iterations as f64;
    [cycles(s.gpr[25], s.gpr[26]), cycles(s.gpr[26], s.gpr[27])]
}

/// The runtime's own step.
fn runtime_step() -> Budget {
    default_budget_for_mode(RuntimeMode::FaultDriven)
}

/// The phases by their sibling.
const STEADY_FDIV: usize = 1;
const STEADY_ADDI: usize = 2;
const FDIV_TO_ADDI: usize = 3;
const ADDI_TO_FDIV: usize = 4;
const FDIV_THEN_ADDI: usize = 5;

/// How far the slowest run of half `h` of phase `p` sits past the mean
/// of its runs on the console, in cycles an iteration.
fn console_slowest_past_mean(w: &[u64], p: usize, h: usize) -> f64 {
    let runs = console_runs(w, p, h);
    runs.iter().copied().fold(0.0, f64::max) - mean(&runs)
}

/// Phase `p`'s replay at one instruction a step and at the runtime's
/// step, run side by side.
fn both_steps(p: usize, iterations: u64) -> ([f64; 2], [f64; 2]) {
    std::thread::scope(|scope| {
        let step = scope.spawn(|| replay(p, iterations, runtime_step()));
        let one = replay(p, iterations, Budget::new(1));
        (one, step.join().expect("the replay at the runtime's step"))
    })
}

#[test]
fn on_the_console_each_half_costs_what_the_sibling_s_class_in_it_costs() {
    // The sibling switches class within its round of the flag: the half
    // of `addi` costs what a steady `addi` sibling costs, and the half of
    // `fdiv` many times more.
    let w = console_words();
    let addi = |h: usize| mean(&console_runs(&w, STEADY_ADDI, h));
    for (p, h) in [(FDIV_TO_ADDI, 1), (ADDI_TO_FDIV, 0)] {
        let switched = mean(&console_runs(&w, p, h));
        assert!(
            (switched - addi(h)).abs() / addi(h) < 0.02,
            "phase {p} half {h}: {switched:.1} cycles an iteration, beside a steady addi \
             sibling {:.1}",
            addi(h),
        );
    }
    for (p, h) in [(FDIV_TO_ADDI, 0), (ADDI_TO_FDIV, 1)] {
        let switched = mean(&console_runs(&w, p, h));
        assert!(
            switched > 3.0 * addi(h),
            "phase {p} half {h}: {switched:.1} cycles an iteration, beside a steady addi \
             sibling {:.1}",
            addi(h),
        );
    }
}

#[test]
fn at_the_runtime_s_step_a_sibling_s_change_of_class_lags_within_the_console_s_spread() {
    // The runtime gives a PPE thread its sibling's pressure once a step,
    // and the sibling sees the flag at its next step.
    let w = console_words();
    let n = half_iterations(&w);
    for p in [FDIV_TO_ADDI, ADDI_TO_FDIV] {
        let (one, step) = both_steps(p, n);
        let lag = (step[1] - one[1]).abs();
        let spread = console_slowest_past_mean(&w, p, 1);
        assert!(
            lag <= spread,
            "phase {p}: the second half takes {:.2} cycles an iteration at the runtime's \
             step and {:.2} at one instruction a step; the console's slowest run sits \
             {spread:.2} past its mean",
            step[1],
            one[1],
        );
    }
}

#[test]
fn beside_a_steady_fdiv_sibling_the_model_times_the_loop_short_of_this_console_capture() {
    // An accepted gap. Beside the steady `fdiv` sibling the console takes
    // about 720 cycles an iteration, and even its fastest run about 350.
    // The model gives each `addi` the shared stall and times about 237.
    // The `ppu_issue_shared` capture times the same loop beside the same
    // sibling at about 195 (`console_ppu_issue_shared.rs`).
    let w = console_words();
    let n = half_iterations(&w);
    let cellgov = replay(STEADY_FDIV, n, Budget::new(1));
    for (h, cellgov) in cellgov.iter().enumerate() {
        let console = mean(&console_runs(&w, STEADY_FDIV, h));
        assert!(
            *cellgov < 0.5 * console,
            "half {h}: CellGov {cellgov:.1} cycles an iteration, the console {console:.1}",
        );
    }
}

#[test]
fn beside_a_sibling_of_fdiv_then_addi_the_model_charges_more_than_the_console() {
    // An accepted gap. A sibling of 8 dependent `fdiv` and then 256
    // independent `addi` a round costs the loop 13% over a steady `addi`
    // sibling on the console: the sibling's `addi` issue while its
    // divides run. The model's sibling waits for each `fdiv` in order at
    // issue and stalls the loop through it: about 80% over at the
    // runtime's step.
    let w = console_words();
    let n = half_iterations(&w);
    let (mixed, addi) = std::thread::scope(|scope| {
        let addi = scope.spawn(|| replay(STEADY_ADDI, n, runtime_step()));
        let mixed = replay(FDIV_THEN_ADDI, n, runtime_step());
        (mixed, addi.join().expect("the steady addi replay"))
    });
    for h in 0..2 {
        let console =
            mean(&console_runs(&w, FDIV_THEN_ADDI, h)) / mean(&console_runs(&w, STEADY_ADDI, h));
        let cellgov = mixed[h] / addi[h];
        assert!(
            console < 1.25 && cellgov > 1.5,
            "half {h}: the mixed sibling costs {console:.2} times a steady addi sibling on \
             the console, {cellgov:.2} in CellGov",
        );
    }
}
