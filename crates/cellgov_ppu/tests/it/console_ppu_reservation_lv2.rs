//! A reservation across a system call, held to the `ppu_reservation_lv2`
//! console capture.
//!
//! Each trial takes a reservation with `lwarx`, runs the case's step, and
//! tries a `stwcx.` on the same word:
//!
//! - case 0 runs nothing between the two;
//! - case 1 calls `sys_time_get_current_time` with its outputs on another
//!   line;
//! - case 2 makes the same call with its outputs inside the reserved
//!   granule;
//! - case 3 calls `sys_ppu_thread_yield`.
//!
//! The console's case 3 yields to a second runnable thread. The replay
//! has no second thread, because CellGov drops the reservation at the call
//! itself, whatever runs next.

use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome};
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/ppu_reservation_lv2/ps3/cech20-cex-493/observation.json";

const CASES: usize = 4;

/// The reserved line.
const BUF: u64 = 0x1000;
/// A line no case reserves.
const ELSEWHERE: u64 = 0x1100;

/// Where the program stores each case's count before it exits.
const RESULT: u64 = 0x1200;

/// The register that counts case `c`'s successful `stwcx.`.
const fn count_reg(c: usize) -> u32 {
    20 + c as u32
}

/// Returns the trial count and each case's CR0[EQ] count from the
/// capture.
fn console() -> (u64, [u64; CASES]) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 4 * (3 + 3 * CASES));
    let word = |i: usize| {
        u64::from(u32::from_be_bytes(
            p[4 * i..4 * i + 4].try_into().expect("4"),
        ))
    };
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");
    let trials = word(2);
    assert_eq!(word(4 + 3), trials, "every time call returned 0");
    assert_eq!(word(4 + 6), trials, "every time call returned 0");
    assert_eq!(
        word(5 + 9),
        trials,
        "the second thread ran in every yield trial"
    );
    (trials, std::array::from_fn(|c| word(3 + 3 * c)))
}

fn addi(rt: u32, ra: u32, si: i16) -> u32 {
    14 << 26 | rt << 21 | ra << 16 | u32::from(si as u16)
}

/// Assembles the probe's trial loop for each case, then stores each
/// count at `RESULT` and calls `sys_process_exit`.
fn program(trials: u64) -> Vec<u32> {
    let mut p = Vec::new();
    // r30 the reserved word, r29 the other line.
    p.push(addi(30, 0, BUF as i16));
    p.push(addi(29, 0, ELSEWHERE as i16));
    for c in 0..CASES {
        p.push(addi(27, 0, trials as i16));
        p.push(0x7C09_03A6 | 27 << 21); // mtctr r27
        let head = p.len();
        p.push(31 << 26 | 5 << 21 | 30 << 11 | 20 << 1); // lwarx r5,0,r30
        let call = |p: &mut Vec<u32>, num: u64, r3: (u32, i16), r4: (u32, i16)| {
            p.push(addi(11, 0, num as i16));
            p.push(addi(3, r3.0, r3.1));
            p.push(addi(4, r4.0, r4.1));
            p.push(0x4400_0002); // sc
        };
        match c {
            1 => call(&mut p, sc::TIME_GET_CURRENT_TIME, (29, 0), (29, 8)),
            2 => call(&mut p, sc::TIME_GET_CURRENT_TIME, (30, 64), (30, 72)),
            3 => call(&mut p, sc::PPU_THREAD_YIELD, (0, 0), (0, 0)),
            _ => {}
        }
        p.push(31 << 26 | 6 << 21 | 30 << 11 | 150 << 1 | 1); // stwcx. r6,0,r30
        p.push(31 << 26 | 7 << 21 | 19 << 1); // mfcr r7
        p.push(21 << 26 | 7 << 21 | 7 << 16 | 3 << 11 | 31 << 6 | 31 << 1); // rlwinm r7,r7,3,31,31
        let n = count_reg(c);
        p.push(31 << 26 | n << 21 | n << 16 | 7 << 11 | 266 << 1); // add rN,rN,r7
        let back = (head as i32 - p.len() as i32) * 4;
        p.push(16 << 26 | 16 << 21 | (back as u32 & 0xFFFC)); // bdnz head
    }
    for c in 0..CASES {
        let d = (RESULT + 4 * c as u64) as u32;
        p.push(36 << 26 | count_reg(c) << 21 | d); // stw rN,d(0)
    }
    p.push(addi(11, 0, sc::PROCESS_EXIT as i16));
    p.push(0x4400_0002); // sc
    p
}

/// Runs the program through the scenario runner and returns each
/// case's count of successful `stwcx.`.
fn run(trials: u64) -> [u64; CASES] {
    let words = program(trials);
    assert!(
        4 * (words.len() as u64) <= BUF,
        "the program ends below the buffers"
    );
    let fixture = ScenarioFixture::builder()
        .memory_size(0x2000)
        .budget(Budget::new(1))
        .max_steps(100_000)
        .seed_memory(move |mem| {
            for (i, w) in words.iter().enumerate() {
                let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).expect("range");
                mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
            }
        })
        .register(|rt| {
            rt.register_unit_with(PpuExecutionUnit::new);
        })
        .build();
    let result = runner::run(fixture);
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.first_fault, None);
    let m = &result.final_memory;
    std::array::from_fn(|c| {
        let at = (RESULT + 4 * c as u64) as usize;
        u64::from(u32::from_be_bytes(m[at..at + 4].try_into().expect("4")))
    })
}

#[test]
fn a_system_call_drops_the_caller_s_reservation_as_on_the_console() {
    let (trials, console) = console();
    assert_eq!(console, [trials, 0, 0, 0]);
    assert_eq!(run(trials), console);
}
