//! CellGov's PPU timing against the `ppu_issue_stall_release` console
//! capture: a store and a reload of its bytes, and divides beside
//! independent loads, alone and beside a sibling of dependent `fdiv`.
//!
//! The capture settles what a sibling's stall does to a queued store's
//! write and to a divide's release of its unit: each waits out the
//! stall, as a pending result does.

use crate::console_ppu_issue_sibling_rate::SIBLINGS;
use crate::console_ppu_issue_timing::{replay, replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_stall_release/ps3/cech20-cex-493/observation.json";

/// The payload's word 65 when the probe wrote every word.
const DONE: u64 = 0x5050_5352;

/// The thread exit value the sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The probe's repetitions of each loop.
const REPS: u64 = 15;

/// Each timed loop's iterations on the console.
const ITERATIONS: u64 = 4096;

/// The probe's loops a phase.
const CHAINS: usize = 7;

/// The two lines r7 and r8 point at.
const LINE_A: u64 = 0x2000;
const LINE_B: u64 = 0x2080;

const STD: u32 = 0xf927_0000;
const LD_A: u32 = 0xe927_0000;
const LD_B: u32 = 0xe928_0000;
const ADDI: [u32; 4] = [0x3a31_0001, 0x3a52_0001, 0x3a73_0001, 0x3a94_0001];
const DIVD: [u32; 4] = [0x7e2f_83d2, 0x7e4f_83d2, 0x7e6f_83d2, 0x7e8f_83d2];
const LD4: [u32; 4] = [0xeaa8_0000, 0xeac8_0008, 0xeae8_0010, 0xeb08_0018];

const SEEDS: &[Seed] = &[
    Seed::Gpr(7, LINE_A),
    Seed::Gpr(8, LINE_B),
    Seed::Gpr(9, 5),
    Seed::Gpr(15, 0x7fff_0000),
    Seed::Gpr(16, 1),
];

/// `std r9,0(r7)`, `k` `addi`, then `ld r9` from the stored bytes or
/// from the other line, four times, then the `bdnz`.
const fn store_then_load<const N: usize>(k: usize, load: u32) -> [u32; N] {
    let mut w = [0u32; N];
    let mut at = 0;
    let mut pass = 0;
    while pass < 4 {
        w[at] = STD;
        at += 1;
        let mut i = 0;
        while i < k {
            w[at] = ADDI[i];
            at += 1;
            i += 1;
        }
        w[at] = load;
        at += 1;
        pass += 1;
    }
    w[at] = 0x4200_0000 | ((4 * at as u32).wrapping_neg() & 0xfffc);
    w
}

/// `head` with r17 to r20 as its target, then the four loads after it,
/// four times, then the `bdnz`.
const fn head_then_loads(head: [u32; 4]) -> [u32; 21] {
    let mut w = [0u32; 21];
    let mut pass = 0;
    while pass < 4 {
        w[5 * pass] = head[pass];
        let mut i = 0;
        while i < 4 {
            w[5 * pass + 1 + i] = LD4[i];
            i += 1;
        }
        pass += 1;
    }
    w[20] = 0x4200_ffb0;
    w
}

const RELOAD: [u32; 9] = store_then_load(0, LD_A);
const OTHER: [u32; 9] = store_then_load(0, LD_B);
const RELOAD_K4: [u32; 25] = store_then_load(4, LD_A);
const OTHER_K4: [u32; 25] = store_then_load(4, LD_B);
const DIVIDES: [u32; 5] = [DIVD[0], DIVD[1], DIVD[2], DIVD[3], 0x4200_fff0];
const DIVD_LD: [u32; 21] = head_then_loads(DIVD);
const ADDI_LD: [u32; 21] = head_then_loads(ADDI);

/// The probe's loops, in its order.
const LOOPS: [Replay; CHAINS] = [
    Replay {
        loop_index: 0,
        words: &RELOAD,
        seeds: SEEDS,
    },
    Replay {
        loop_index: 1,
        words: &OTHER,
        seeds: SEEDS,
    },
    Replay {
        loop_index: 2,
        words: &RELOAD_K4,
        seeds: SEEDS,
    },
    Replay {
        loop_index: 3,
        words: &OTHER_K4,
        seeds: SEEDS,
    },
    Replay {
        loop_index: 4,
        words: &DIVIDES,
        seeds: SEEDS,
    },
    Replay {
        loop_index: 5,
        words: &DIVD_LD,
        seeds: SEEDS,
    },
    Replay {
        loop_index: 6,
        words: &ADDI_LD,
        seeds: SEEDS,
    },
];

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 752);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[93], DONE, "the probe wrote every word");
    assert_eq!(words[92], REPS, "fifteen runs a loop");
    assert_eq!(words[86], 0, "the sibling was created");
    assert_eq!(words[87], 0, "the sibling joined");
    assert_eq!(words[88], SIBLING_EXIT, "the sibling exited");
    assert_eq!(words[89], 1, "the sibling ran");
    assert_eq!(words[90], 0, "the sleep returned");
    for l in 0..2 * CHAINS {
        assert_eq!(
            words[56 + l],
            REPS,
            "loop {l} timed fifteen runs that count"
        );
    }
    words
}

/// Loop `chain`'s mean cycles an iteration in phase `phase` on the
/// console, over the runs that count.
fn console_mean(w: &[u64], phase: usize, chain: usize) -> f64 {
    let at = CHAINS * phase + chain;
    (w[42 + at] * PPU_CYCLES_PER_TB_TICK) as f64 / (w[56 + at] * ITERATIONS) as f64
}

fn sibling(name: &str) -> &'static Sibling {
    &SIBLINGS
        .iter()
        .find(|(n, _)| *n == name)
        .expect("a ppu_issue_sibling sibling")
        .1
}

/// The sibling's phase.
const FDIV: usize = 1;

/// Loop `chain`'s cycles an iteration in CellGov: alone in phase 0,
/// beside the `fdiv` sibling in phase 1.
fn cellgov(phase: usize, chain: usize) -> f64 {
    let r = &LOOPS[chain];
    let cycles = if phase == 0 {
        replay(r, 0)
    } else {
        replay_beside(r, 1, sibling("fdiv"))
    };
    cycles.unwrap_or_else(|| panic!("loop {chain} replays"))
}

/// The cycles an iteration loop `a` takes past loop `b`, on the console
/// and in CellGov, in phase `phase`.
fn excess(w: &[u64], phase: usize, a: usize, b: usize) -> (f64, f64) {
    (
        console_mean(w, phase, a) - console_mean(w, phase, b),
        cellgov(phase, a) - cellgov(phase, b),
    )
}

#[test]
fn the_sibling_kept_its_rate_through_each_counted_run() {
    let w = console_words();
    let alone_rate = w[84] as f64 / w[85] as f64;
    for c in 0..CHAINS {
        let at = CHAINS * FDIV + c;
        let kept = (w[28 + at] as f64 / w[42 + at] as f64) / alone_rate;
        assert!(
            kept > 0.5,
            "loop {c}: the sibling kept {kept:.2} of its rate"
        );
    }
}

#[test]
fn alone_a_reload_of_queued_bytes_waits_for_the_store_s_write() {
    // A reload of the bytes a store just queued takes 196.5 and 207.7
    // cycles an iteration; the model's store queue gives 196 to each.
    let w = console_words();
    for chain in [0, 2] {
        let console = console_mean(&w, 0, chain);
        let cellgov = cellgov(0, chain);
        assert!(
            (cellgov - console).abs() / console < 0.10,
            "loop {chain}: CellGov {cellgov:.1} cycles an iteration, the console {console:.1}",
        );
    }
}

#[test]
fn beside_a_stalling_sibling_a_reload_still_waits_for_the_queued_store() {
    // Beside the `fdiv` sibling the reload costs more over a load of the
    // other line than it does alone: 847 cycles an iteration with no
    // `addi` between the store and the load, and 1402 with four, against
    // about 170 alone. The four `addi` stall long enough for a store
    // whose write does not move to finish before the load; the model
    // then gives the reload no cost. Each sibling stall pushes back the
    // queued store's write.
    let w = console_words();
    for (reload, other) in [(0, 1), (2, 3)] {
        let (console_alone, cellgov_alone) = excess(&w, 0, reload, other);
        let (console_beside, cellgov_beside) = excess(&w, FDIV, reload, other);
        assert!(
            console_beside > console_alone,
            "loop {reload}: the console's reload costs {console_beside:.1} beside the \
             sibling, {console_alone:.1} alone",
        );
        assert!(
            cellgov_beside > 0.9 * cellgov_alone,
            "loop {reload}: CellGov's reload costs {cellgov_beside:.1} beside the \
             sibling, {cellgov_alone:.1} alone",
        );
    }
}

#[test]
fn beside_a_stalling_sibling_a_divide_still_holds_its_unit() {
    // A `divd` with four independent `ld` after it costs 196 cycles over
    // an `addi` with the same loads alone, and 143 beside the `fdiv`
    // sibling, where the four loads stall far longer than the divide
    // runs. A divide whose release of the FXU does not move would cost
    // nothing there. Each sibling stall pushes back the unit's release.
    // The runs beside the sibling spread wide (the `addi` loop's from 106
    // to 1096 cycles an iteration), so the bound is half the cost alone.
    let w = console_words();
    let (console_alone, cellgov_alone) = excess(&w, 0, 5, 6);
    let (console_beside, cellgov_beside) = excess(&w, FDIV, 5, 6);
    assert!(
        console_beside > 0.5 * console_alone,
        "the console's divide costs {console_beside:.1} beside the sibling, \
         {console_alone:.1} alone",
    );
    assert!(
        cellgov_beside > 0.5 * cellgov_alone,
        "CellGov's divide costs {cellgov_beside:.1} beside the sibling, {cellgov_alone:.1} alone",
    );
}

#[test]
fn beside_a_stalling_sibling_the_model_times_back_to_back_divides_short_of_the_console() {
    // An accepted gap. Four independent `divd` take 211 cycles an
    // iteration alone and 671 beside the `fdiv` sibling: each divide
    // waits the one before it and then a stall of about 115 cycles. The
    // model's stall beside this sibling is shorter, as it is in
    // `console_ppu_issue_miss.rs`. The sibling kept its rate through
    // 15 of the 62 runs this loop made, the fewest of any loop.
    let w = console_words();
    let added = |alone: f64, beside: f64| (beside - alone) / 4.0;
    let console = added(console_mean(&w, 0, 4), console_mean(&w, FDIV, 4));
    let cellgov = added(cellgov(0, 4), cellgov(FDIV, 4));
    assert!(
        cellgov > 0.0 && cellgov < console,
        "CellGov adds {cellgov:.1} cycles a divide beside the sibling, the console {console:.1}",
    );
}
