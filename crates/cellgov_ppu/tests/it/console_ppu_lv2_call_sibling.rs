//! The `ppu_lv2_call_sibling` console capture against the round trips
//! CellGov charges: the cheapest call and a yield that does not switch
//! threads, alone and beside a sibling thread that runs `addi`, makes the
//! same cheap call, or yields.

use crate::console_ppu_issue_timing::{replay, replay_beside, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str = "../../tests/micro/ppu_lv2_call_sibling/ps3/cech20-cex-493/observation.json";

/// The payload's word 62 when the probe wrote every word.
const DONE: u64 = 0x5050_4C53;

/// The calls a run makes.
const CALLS: u64 = 1 << 12;

/// The runs of each loop.
const REPS: u64 = 5;

/// The phases: no sibling, then the three siblings.
const PHASES: usize = 4;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The calls each phase times, in the probe's order.
const CALLS_TIMED: [u64; 2] = [sc::TIME_GET_TIMEBASE_FREQUENCY, sc::PPU_THREAD_YIELD];

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 512);
    let w: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[62], DONE, "the probe wrote every word");
    assert_eq!((w[60], w[61]), (REPS, CALLS));
    for p in 1..PHASES {
        let m = 40 + 4 * p;
        assert_eq!(
            (w[m], w[m + 1], w[m + 2], w[m + 3]),
            (0, 0, 1, SIBLING_EXIT),
            "phase {p}: the sibling's create rc, join rc, start and exit value",
        );
    }
    w
}

/// Loop `call` of phase `phase`: its fastest run, in core cycles a call.
fn console_cycles(w: &[u64], phase: usize, call: usize) -> f64 {
    (w[2 * (2 * phase + call)] * PPU_CYCLES_PER_TB_TICK) as f64 / CALLS as f64
}

/// Where a replay's sibling keeps the probe's `rounds` count and its
/// `stop` flag, each on its own line. The flag stays zero.
const ROUNDS: u64 = 0x3000;
const STOP: u64 = 0x3080;

/// The probe's `calls` loop body, from its `li r3,0` to its `bne+`: zero
/// arguments, the number from r29, the `sc`, the return-code check on
/// cr4, and the count in r31. As in `console_ppu_lv2_call_cost`, the
/// replay starts with cr4 clear and skips the check.
const CALL_LOOP: [u32; 16] = [
    0x3860_0000,
    0x3880_0000,
    0x38a0_0000,
    0x38c0_0000,
    0x38e0_0000,
    0x3900_0000,
    0x3920_0000,
    0x3940_0000,
    0x7fab_eb78,
    0x4400_0002,
    0x4092_000c,
    0x787e_0020,
    0x2e3e_0000,
    0x2c3f_0001,
    0x3bff_ffff,
    0x4082_ffc4,
];

/// The probe's phase-1 sibling loop: 64 independent `addi`, the load of
/// `stop` from r10 and its check, the `rounds` count at r8, and the
/// branch back.
const ADDI_SIBLING_LOOP: [u32; 71] = {
    let addi = [0x3a31_0001, 0x3a52_0001, 0x3a73_0001, 0x3a94_0001];
    let tail = [
        0x800a_0000,
        0x2c00_0000,
        0x4082_0014,
        0xe928_0000,
        0x3929_0001,
        0xf928_0000,
        0x4bff_fee8,
    ];
    let mut words = [0; 71];
    let mut i = 0;
    while i < 64 {
        words[i] = addi[i % 4];
        i += 1;
    }
    while i < 71 {
        words[i] = tail[i - 64];
        i += 1;
    }
    words
};

/// The probe's phase-2 and phase-3 sibling loop: zero arguments, the
/// number from r31, the `sc`, the `rounds` count at r30, and the load of
/// `stop` from r29 and its check.
const CALLING_SIBLING_LOOP: [u32; 16] = [
    0x3860_0000,
    0x3880_0000,
    0x38a0_0000,
    0x38c0_0000,
    0x38e0_0000,
    0x3900_0000,
    0x3920_0000,
    0x3940_0000,
    0x7feb_fb78,
    0x4400_0002,
    0xe93e_0000,
    0x3929_0001,
    0xf93e_0000,
    0x813d_0000,
    0x2fa9_0000,
    0x419e_ffc4,
];

/// The probe's sibling of `addi`.
fn addi_sibling() -> Sibling {
    Sibling {
        words: &ADDI_SIBLING_LOOP,
        seeds: &[Seed::Gpr(8, ROUNDS), Seed::Gpr(10, STOP)],
    }
}

/// The probe's sibling that makes syscall `number` in a loop.
fn calling_sibling(number: u64) -> Sibling {
    Sibling {
        words: &CALLING_SIBLING_LOOP,
        seeds: Box::leak(
            vec![
                Seed::Gpr(29, STOP),
                Seed::Gpr(30, ROUNDS),
                Seed::Gpr(31, number),
            ]
            .into_boxed_slice(),
        ),
    }
}

/// CellGov's cycles a call of loop `call` in phase `phase`.
fn cellgov_cycles(phase: usize, call: usize) -> f64 {
    let r = Replay {
        loop_index: call,
        words: &CALL_LOOP,
        seeds: Box::leak(
            vec![Seed::Gpr(29, CALLS_TIMED[call]), Seed::Counter(31)].into_boxed_slice(),
        ),
    };
    let cycles = match phase {
        0 => replay(&r, 0),
        1 => replay_beside(&r, 1, &addi_sibling()),
        2 => replay_beside(&r, 1, &calling_sibling(sc::TIME_GET_TIMEBASE_FREQUENCY)),
        _ => replay_beside(&r, 1, &calling_sibling(sc::PPU_THREAD_YIELD)),
    };
    cycles.unwrap_or_else(|| panic!("phase {phase} loop {call} replays"))
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.05;

#[test]
fn each_loop_lies_within_the_band() {
    let w = console_words();
    let misses: Vec<String> = (0..PHASES)
        .flat_map(|p| (0..2).map(move |c| (p, c)))
        .map(|(p, c)| (p, c, cellgov_cycles(p, c), console_cycles(&w, p, c)))
        .filter(|&(.., cellgov, console)| (cellgov - console).abs() / console > BAND)
        .map(|(p, c, cellgov, console)| {
            format!(
                "phase {p} loop {c}: CellGov {cellgov:.1} cycles a call, the console's \
                 fastest {console:.1}"
            )
        })
        .collect();
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

#[test]
fn a_sibling_in_lv2_calls_slows_a_call_far_more_than_a_busy_sibling_in_user_mode() {
    // The console's ratios to the call alone: 1.18 and 1.25 beside the
    // `addi` sibling, 1.75 and 1.86 beside the sibling of cheap calls.
    let w = console_words();
    for (who, cycles) in [
        (
            "the console",
            &(|p, c| console_cycles(&w, p, c)) as &dyn Fn(usize, usize) -> f64,
        ),
        ("CellGov", &cellgov_cycles),
    ] {
        for call in 0..2 {
            let alone = cycles(0, call);
            let (user, kernel) = (cycles(1, call) / alone, cycles(2, call) / alone);
            assert!(
                (1.1..1.35).contains(&user) && (1.6..1.95).contains(&kernel),
                "{who}, loop {call}: {user:.2} times the call alone beside a sibling of addi, \
                 {kernel:.2} beside a sibling of calls",
            );
        }
    }
}
