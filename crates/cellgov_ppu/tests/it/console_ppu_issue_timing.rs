//! CellGov's PPU timing against the `ppu_issue_timing` probe's console
//! capture.
//!
//! The probe times twenty-three loops with `mftb`, five runs each. The
//! constants of `cellgov_ps3_abi::hw::ppu_issue` are the capture's
//! fastest runs, and the first test derives each one again. The second
//! test runs loops 0 to 18 through the execution unit and records the
//! gap: the PPU charges one nanosecond a retired instruction, so no loop
//! takes the console's cycles yet. Loops 19 to 22 need the LV2 host and a
//! second hardware thread, which the PPU model does not have.

use cellgov_core::Runtime;
use cellgov_exec::UnitStatus;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_issue::*;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/ppu_issue_timing/ps3/cech20-cex-493/observation.json";

/// log2 of each loop's iterations on the console, from the probe's
/// header.
const ITERATIONS_LOG2: [u32; 23] = [
    20, 20, 20, 18, 16, 18, 18, 18, 20, 20, 20, 17, 15, 12, 18, 20, 20, 20, 20, 12, 20, 20, 17,
];

/// The payload's word 55 when the probe wrote every word.
const DONE: u64 = 0x5050_4954;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 448);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[55], DONE, "the probe wrote every word");
    assert_eq!(words[54], 5, "five runs a loop");
    assert_eq!(words[47], 0, "the sibling thread was created");
    assert_ne!(words[48], u64::MAX, "the sibling thread ran");
    assert_eq!(words[50], 0, "the sibling thread joined");
    assert_eq!(
        words[51], 0x5157,
        "the sibling thread exited through its own path"
    );
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration, unrounded.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2[k]) as f64
}

fn round(v: f64) -> u64 {
    v.round() as u64
}

#[test]
fn the_recorded_constants_are_the_captures() {
    let w = console_words();
    assert_eq!(w[46], cellgov_ps3_abi::hw::ppu::CELL_PPU_TIMEBASE_HZ);

    // At 40 cycles a tick the three steadiest loops read whole cycles to
    // half a percent; at 39 or 41 the empty loop does not.
    for k in [0, 1, 11] {
        let c = console_cycles(&w, k);
        assert!(
            (c - c.round()).abs() / c < 0.005,
            "loop {k} reads {c} cycles"
        );
    }
    for ratio in [39, 41] {
        let c = (w[0] * ratio) as f64 / (1u64 << ITERATIONS_LOG2[0]) as f64;
        assert!(
            (c - c.round()).abs() / c > 0.01,
            "at {ratio} the empty loop reads {c} cycles"
        );
    }

    let per = |k: usize, n: f64| round(console_cycles(&w, k) / n);
    assert_eq!(per(0, 1.0), PPU_LOOP_BRANCH_CYCLES);
    assert_eq!(per(2, 8.0), PPU_FX_LATENCY_CYCLES);
    assert_eq!(per(3, 8.0), PPU_MULLD_LATENCY_CYCLES);
    assert_eq!(per(4, 4.0), PPU_DIVD_LATENCY_CYCLES);
    assert_eq!(per(5, 8.0), PPU_FP_LATENCY_CYCLES);
    assert_eq!(per(6, 8.0), PPU_FP_LATENCY_CYCLES);
    assert_eq!(per(7, 8.0), PPU_VMX_FLOAT_LATENCY_CYCLES);
    assert_eq!(per(11, 8.0), PPU_L1_LOAD_USE_CYCLES);
    assert_eq!(per(12, 8.0), PPU_L2_LOAD_USE_CYCLES);
    assert_eq!(per(13, 8.0), PPU_MEMORY_LOAD_USE_CYCLES);
    assert_eq!(per(14, 4.0), PPU_LOAD_HIT_STORE_CYCLES);
    assert_eq!(per(16, 8.0), PPU_TAKEN_BRANCH_CYCLES);
    assert_eq!(
        round((console_cycles(&w, 18) - console_cycles(&w, 17)) * 2.0),
        PPU_BRANCH_MISS_CYCLES,
    );
    assert_eq!(per(19, 1.0), PPU_SYSCALL_ROUND_TRIP_CYCLES);
    assert_eq!((per(1, 1.0), per(20, 1.0)), PPU_SMT_INDEPENDENT_ADDI_CYCLES);
    assert_eq!((per(2, 1.0), per(21, 1.0)), PPU_SMT_DEPENDENT_ADD_CYCLES);
    assert_eq!((per(11, 1.0), per(22, 1.0)), PPU_SMT_L1_CHASE_CYCLES);
}

/// Where each replay places its loop, its one data line and its rings.
const CODE: u64 = 0x1000;
const ONE_LINE: u64 = 0x2000;
const L1_RING: u64 = 0x1_0000;
const L2_RING: u64 = 0x2_0000;
const MEM_RING: u64 = 0x10_0000;
const LINE: u64 = 128;

/// Iterations of each replay. The PPU's charge is linear in retired
/// instructions, so a short run gives the same cycles an iteration.
const REPLAY_ITERATIONS: u64 = 512;

/// A register a replay seeds before its loop.
#[derive(Clone, Copy)]
enum Seed {
    Gpr(usize, u64),
    Fpr(usize, u64),
    /// A GPR holding the head of a ring of `bytes` at `base`, shuffled or
    /// strided as the probe builds it.
    Ring(usize, u64, u64, bool),
}

const ONE: u64 = 0x3FF0_0000_0000_0000;

/// One loop of the probe: its body as `powerpc64-ps3-elf-objdump` of the
/// built image prints it, from the aligned label through the `bdnz`, and
/// the registers the body reads.
struct Replay {
    loop_index: usize,
    words: &'static [u32],
    seeds: &'static [Seed],
}

const BDNZ_8: u32 = 0x4200_ffe0;

const REPLAYS: [Replay; 19] = [
    Replay {
        loop_index: 0,
        words: &[0x4200_0000],
        seeds: &[],
    },
    Replay {
        loop_index: 1,
        words: &[
            0x39ce_0001,
            0x39ef_0001,
            0x3a10_0001,
            0x3a31_0001,
            0x3a52_0001,
            0x3a73_0001,
            0x3a94_0001,
            0x3ab5_0001,
            BDNZ_8,
        ],
        seeds: &[],
    },
    Replay {
        loop_index: 2,
        words: &eight_then_bdnz(0x7d29_7a14),
        seeds: &[Seed::Gpr(9, 1), Seed::Gpr(15, 3)],
    },
    Replay {
        loop_index: 3,
        words: &eight_then_bdnz(0x7d29_79d2),
        seeds: &[Seed::Gpr(9, 7), Seed::Gpr(15, 1)],
    },
    Replay {
        loop_index: 4,
        words: &[
            0x7d29_7bd2,
            0x7d29_7bd2,
            0x7d29_7bd2,
            0x7d29_7bd2,
            0x4200_fff0,
        ],
        seeds: &[Seed::Gpr(9, 0x1_2345_6789), Seed::Gpr(15, 1)],
    },
    Replay {
        loop_index: 5,
        words: &eight_then_bdnz(0xfc00_602a),
        seeds: &[Seed::Fpr(0, ONE), Seed::Fpr(12, 0)],
    },
    Replay {
        loop_index: 6,
        words: &eight_then_bdnz(0xfc00_603a),
        seeds: &[Seed::Fpr(0, ONE), Seed::Fpr(12, 0)],
    },
    Replay {
        loop_index: 7,
        words: &eight_then_bdnz(0x1042_180a),
        seeds: &[],
    },
    Replay {
        loop_index: 8,
        words: &[
            0x39ce_0001,
            0xea48_0000,
            0x39ef_0001,
            0xea68_0008,
            0x3a10_0001,
            0xea88_0010,
            0x3a31_0001,
            0xeaa8_0018,
            BDNZ_8,
        ],
        seeds: &[Seed::Gpr(8, ONE_LINE)],
    },
    Replay {
        loop_index: 9,
        words: &[
            0x39ce_0001,
            0xfc21_282a,
            0x39ef_0001,
            0xfc42_282a,
            0x3a10_0001,
            0xfc63_282a,
            0x3a31_0001,
            0xfc84_282a,
            BDNZ_8,
        ],
        seeds: &[
            Seed::Fpr(1, ONE),
            Seed::Fpr(2, ONE),
            Seed::Fpr(3, ONE),
            Seed::Fpr(4, ONE),
            Seed::Fpr(5, 0),
        ],
    },
    Replay {
        loop_index: 10,
        words: &[
            0xfc21_482a,
            0xfc42_482a,
            0xfc63_482a,
            0xfc84_482a,
            0xfca5_482a,
            0xfcc6_482a,
            0xfce7_482a,
            0xfd08_482a,
            BDNZ_8,
        ],
        seeds: &[
            Seed::Fpr(1, ONE),
            Seed::Fpr(2, ONE),
            Seed::Fpr(3, ONE),
            Seed::Fpr(4, ONE),
            Seed::Fpr(5, ONE),
            Seed::Fpr(6, ONE),
            Seed::Fpr(7, ONE),
            Seed::Fpr(8, ONE),
            Seed::Fpr(9, 0),
        ],
    },
    Replay {
        loop_index: 11,
        words: &eight_then_bdnz(0xe884_0000),
        seeds: &[Seed::Ring(4, L1_RING, 16 * 1024, false)],
    },
    Replay {
        loop_index: 12,
        words: &eight_then_bdnz(0xe884_0000),
        seeds: &[Seed::Ring(4, L2_RING, 256 * 1024, true)],
    },
    Replay {
        loop_index: 13,
        words: &eight_then_bdnz(0xe884_0000),
        seeds: &[Seed::Ring(4, MEM_RING, 8 * 1024 * 1024, true)],
    },
    Replay {
        loop_index: 14,
        words: &[
            0xf927_0000,
            0xe927_0000,
            0xf927_0000,
            0xe927_0000,
            0xf927_0000,
            0xe927_0000,
            0xf927_0000,
            0xe927_0000,
            BDNZ_8,
        ],
        seeds: &[Seed::Gpr(9, 5), Seed::Gpr(7, ONE_LINE)],
    },
    Replay {
        loop_index: 15,
        words: &[
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x4200_ffc0,
        ],
        seeds: &[Seed::Gpr(14, 0)],
    },
    Replay {
        loop_index: 16,
        words: &eight_then_bdnz(0x4800_0004),
        seeds: &[],
    },
    Replay {
        loop_index: 17,
        words: &LCG_LOOP,
        seeds: &[
            Seed::Gpr(14, 1),
            Seed::Gpr(16, 0x5851_f42d),
            Seed::Gpr(9, 0),
            Seed::Gpr(3, 0),
        ],
    },
    Replay {
        loop_index: 18,
        words: &LCG_LOOP,
        seeds: &[
            Seed::Gpr(14, 1),
            Seed::Gpr(16, 0x5851_f42d),
            Seed::Gpr(9, 0),
            Seed::Gpr(3, 1),
        ],
    },
];

const LCG_LOOP: [u32; 8] = [
    0x7dce_81d2,
    0x39ce_0001,
    0x79d1_0fe0,
    0x7e31_1838,
    0x2c31_0000,
    0x4182_0008,
    0x3929_0001,
    0x4200_ffe4,
];

/// The body of an eight-instruction loop: the word eight times, then the
/// loop's `bdnz`.
const fn eight_then_bdnz(word: u32) -> [u32; 9] {
    [word, word, word, word, word, word, word, word, BDNZ_8]
}

fn write_u64(mem: &mut GuestMemory, at: u64, v: u64) {
    let range = ByteRange::new(GuestAddr::new(at), 8).expect("range");
    mem.apply_commit(range, &v.to_be_bytes()).expect("mapped");
}

/// The probe's `build_ring`: one doubleword a 128-byte line, linked in
/// order or in its LCG-shuffled order. Returns the head.
fn build_ring(mem: &mut GuestMemory, base: u64, bytes: u64, shuffle: bool) -> u64 {
    let n = (bytes / LINE) as usize;
    let mut order: Vec<u64> = (0..n as u64).collect();
    if shuffle {
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        for i in (1..n).rev() {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let j = ((seed >> 33) % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
    }
    for i in 0..n {
        let next = base + order[(i + 1) % n] * LINE;
        write_u64(mem, base + order[i] * LINE, next);
    }
    base + order[0] * LINE
}

/// Run one replay's loop `REPLAY_ITERATIONS` times, one instruction a
/// step, and return the unit's clock when the loop exits, in cycles, or
/// `None` when the unit faults.
fn replay(r: &Replay) -> Option<u64> {
    let words = r.words;
    let ring_end = r
        .seeds
        .iter()
        .filter_map(|s| match s {
            Seed::Ring(_, base, bytes, _) => Some(base + bytes),
            _ => None,
        })
        .max()
        .unwrap_or(L1_RING);
    let mut mem = GuestMemory::new(ring_end as usize);
    for (i, w) in words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(CODE + 4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    let mut gprs = Vec::new();
    for seed in r.seeds {
        if let Seed::Ring(reg, base, bytes, shuffle) = *seed {
            gprs.push((reg, build_ring(&mut mem, base, bytes, shuffle)));
        }
    }
    let exit = CODE + 4 * words.len() as u64;

    let mut rt = Runtime::new(mem, Budget::new(1), usize::MAX);
    let seeds = r.seeds;
    let unit = rt.register_unit_with(|id| {
        let mut ppu = PpuExecutionUnit::new(id);
        let state = ppu.state_mut();
        state.pc = CODE;
        state.set_ctr(REPLAY_ITERATIONS);
        for seed in seeds {
            match *seed {
                Seed::Gpr(k, v) => state.set_gpr(k, v),
                Seed::Fpr(k, v) => state.set_fpr(k, v),
                Seed::Ring(..) => {}
            }
        }
        for &(k, v) in &gprs {
            state.set_gpr(k, v);
        }
        ppu
    });

    let pc = |rt: &Runtime| -> u64 {
        rt.registry()
            .get(unit)
            .and_then(|u| u.as_any().downcast_ref::<PpuExecutionUnit>())
            .expect("the PPU unit")
            .state()
            .pc
    };
    let bound = REPLAY_ITERATIONS * (words.len() as u64 + 1) + 16;
    for _ in 0..bound {
        if pc(&rt) == exit {
            break;
        }
        if rt.registry().effective_status(unit) == Some(UnitStatus::Faulted) {
            return None;
        }
        let step = rt
            .step()
            .unwrap_or_else(|e| panic!("loop {} at pc {:#x}: {e:?}", r.loop_index, pc(&rt)));
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    assert_eq!(pc(&rt), exit, "loop {} ran to its exit", r.loop_index);
    Some(rt.registry().clock(unit).raw())
}

/// The loop whose body the interpreter refuses: `vaddfp`.
const REFUSED_LOOP: usize = 7;

#[test]
fn today_s_model_misses_every_loop_the_console_times() {
    let w = console_words();
    let mut gaps = Vec::new();
    for r in &REPLAYS {
        let Some(clock) = replay(r) else {
            assert_eq!(
                r.loop_index, REFUSED_LOOP,
                "loop {} faults in the interpreter",
                r.loop_index,
            );
            continue;
        };
        assert_ne!(
            r.loop_index, REFUSED_LOOP,
            "the interpreter now executes vaddfp, so loop 7 joins the gap rows",
        );
        let cellgov = round(clock as f64 / REPLAY_ITERATIONS as f64);
        let console = round(console_cycles(&w, r.loop_index));
        gaps.push((r.loop_index, cellgov, console));
    }
    assert_eq!(
        gaps.len(),
        18,
        "every replayed loop but the refused one has a row"
    );
    for (k, cellgov, console) in &gaps {
        assert_ne!(
            cellgov, console,
            "loop {k} takes the console's {console} cycles an iteration; the PPU model \
             now times it, so this row of the gap is closed: {gaps:?}",
        );
    }
}
