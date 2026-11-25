//! CellGov's runs of the `ppu_lv2_call_contended` and
//! `ppu_lv2_call_woken_forms` microtests, held to their console captures.
//! Two or three PPU threads run through the runtime, the LV2 host and the
//! issue model, and the probes' own `mftb` loops time them.
//!
//! The file compiles only under the `microtests` feature. That feature
//! declares the microtest tree built, because the probes' ELFs are
//! gitignored build output.
//!
//! The contended probe times five loops:
//!
//! - the cheapest call
//! - a yield alone
//! - a yield beside one yielding helper
//! - a yield among two yielding helpers
//! - a semaphore handoff with one helper
//!
//! The woken-forms probe times the cheapest call, then a handoff with one
//! helper through each of a semaphore, an event flag, an event queue and
//! a condition variable. On each side, a loop costs its own cycles beyond
//! the round trip of the cheapest call. Each test subtracts that side's
//! own cycles from every loop before it compares the two sides.
//!
//! The `ppu_lv2_call_preempted_forms` probe's tests are in
//! [`preempted_forms`]. The `ppu_lv2_call_wait_ends` probe's tests are
//! in [`wait_ends`].

#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

mod preempted_forms;
mod wait_ends;

use std::path::{Path, PathBuf};
use std::process::Command;

use cellgov_compare::Observation;
use cellgov_ps3_abi::hw::ppu_issue::{PPU_CYCLES_PER_TB_TICK, PPU_SYSCALL_ROUND_TRIP_CYCLES};

const PROBE: &str = "ppu_lv2_call_contended";

const WOKEN_PROBE: &str = "ppu_lv2_call_woken_forms";

/// Calls or handoffs a loop makes.
const CALLS: u64 = 1 << 12;

/// The payload's word 60 when the probe wrote every word.
const DONE: u64 = 0x5050_4C59;

/// The woken-forms payload's word 90 when the probe wrote every word.
const WOKEN_DONE: u64 = 0x5050_4C57;

/// The loops the probe times.
const LOOPS: usize = 5;

/// The loop of the cheapest call.
const FREQUENCY_LOOP: usize = 0;

/// Retired instructions the run may take: each yield ends a step.
const MAX_STEPS: u64 = 2_560_000_000;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The probe's payload as big-endian `u64` words, from the `CGOV` frame
/// in `tty`.
fn cgov_words(tty: &[u8]) -> Vec<u64> {
    let at = tty
        .windows(4)
        .position(|w| w == b"CGOV")
        .expect("the run wrote a CGOV frame");
    let len = u32::from_be_bytes(tty[at + 4..at + 8].try_into().unwrap()) as usize;
    tty[at + 8..at + 8 + len]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().unwrap()))
        .collect()
}

/// Boot `probe` and return its payload.
///
/// The microtest's manifest declares no title, so the run boots from a
/// title manifest that names the probe's build directory. The test
/// writes that manifest beside the run's observation.
fn cellgov_words(probe: &str) -> Vec<u64> {
    let root = workspace_root();
    let scratch = root
        .join("target")
        .join("lv2_call_contended_microtest")
        .join(format!("{probe}-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let manifest = scratch.join("title.toml");
    std::fs::write(
        &manifest,
        format!(
            "[cellgov.title]\nshort_name = \"contended\"\ndisplay_name = \"{probe} probe\"\n\
             eboot_candidates = [\"{probe}.elf\"]\nyear = 2026\ndeveloper = \"CellGov\"\n\
             engine = \"microtest\"\ndistribution = \"microtest\"\n\n\
             [cellgov.source]\nkind = \"manifest-relative\"\npath = \"../../../tests/micro/{probe}/build\"\n\n\
             [cellgov.checkpoint]\nkind = \"process-exit\"\n"
        ),
    )
    .unwrap();
    let observation = scratch.join("observation.json");
    std::fs::remove_file(&observation).ok();
    let output = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["boot", "run", "--title-manifest"])
        .arg(&manifest)
        .arg("--max-steps")
        .arg(MAX_STEPS.to_string())
        .arg("--save-observation")
        .arg(&observation)
        .env("CELLGOV_NO_FIRMWARE_DIR", "1")
        .current_dir(&root)
        .output()
        .expect("spawn cellgov boot run");
    assert!(
        output.status.success(),
        "cellgov boot run exited non-zero:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let json = std::fs::read_to_string(&observation).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\nthe microtests feature declares the microtest tree built; \
             build it with tests/micro/{probe}/build.sh",
            observation.display()
        )
    });
    let observation: Observation = serde_json::from_str(&json).unwrap();
    cgov_words(&observation.tty_log)
}

/// `probe`'s console capture's payload as big-endian `u64` words.
fn console_words(probe: &str) -> Vec<u64> {
    let path = workspace_root()
        .join("tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/observation.json");
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    observation.memory_regions[0]
        .data
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().unwrap()))
        .collect()
}

/// Loop `l`'s fastest and slowest runs, in core cycles a call.
fn cycles(w: &[u64], l: usize) -> (f64, f64) {
    let per = |ticks: u64| (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / CALLS as f64;
    (per(w[2 * l]), per(w[2 * l + 1]))
}

/// The cycles a loop of `w` costs beyond the round trip of the cheapest
/// call.
fn rest(w: &[u64]) -> f64 {
    cycles(w, FREQUENCY_LOOP).0.round() - PPU_SYSCALL_ROUND_TRIP_CYCLES as f64
}

#[test]
fn each_contended_loop_lies_within_the_console_s_spread() {
    let console = console_words(PROBE);
    let cellgov = cellgov_words(PROBE);
    for (side, w) in [("console", &console), ("cellgov", &cellgov)] {
        assert_eq!(w[60], DONE, "{side}: the probe wrote every word");
        assert_eq!(w[57], 0, "{side}: every setup and teardown step returned 0");
        for (word, &rc) in (32..).zip(&w[32..44]) {
            assert_eq!(rc, 0, "{side}: step word {word}");
        }
    }
    // The cheapest call's loop sets `rest`, so it matches by construction.
    for l in FREQUENCY_LOOP + 1..LOOPS {
        let (fastest, slowest) = cycles(&console, l);
        let console_call = fastest - rest(&console);
        let cellgov_call = cycles(&cellgov, l).0 - rest(&cellgov);
        assert!(
            (cellgov_call - console_call).abs() <= slowest - fastest,
            "loop {l}: CellGov {cellgov_call:.0} cycles a call beside the console's \
             {console_call:.0}, spread {:.0}",
            slowest - fastest,
        );
    }
}

#[test]
fn each_woken_handoff_lies_within_the_console_s_spread() {
    // In loops 1 to 4, every wait of the main thread blocks until the
    // helper wakes it.
    let console = console_words(WOKEN_PROBE);
    let cellgov = cellgov_words(WOKEN_PROBE);
    for (side, w) in [("console", &console), ("cellgov", &cellgov)] {
        assert_eq!(w[90], WOKEN_DONE, "{side}: the probe wrote every word");
        assert_eq!(w[88], 0, "{side}: every setup and teardown step returned 0");
        // Word 16 holds the cheapest call's return, the time-base
        // frequency. Words 17 to 20 hold the handoff loops' return codes.
        for (word, &rc) in (17..21).chain(32..68).chain(72..80).chain(84..86).zip(
            w[17..21]
                .iter()
                .chain(&w[32..68])
                .chain(&w[72..80])
                .chain(&w[84..86]),
        ) {
            assert_eq!(rc, 0, "{side}: word {word}");
        }
        for (form, &n) in w[80..84].iter().enumerate() {
            assert_eq!(n, 5 * CALLS, "{side}: helper {form}'s handoffs");
        }
    }
    // The test checks every loop before it fails, so one run names each
    // form whose round trip misses the spread.
    let misses: Vec<String> = (FREQUENCY_LOOP + 1..LOOPS)
        .filter_map(|l| {
            let (fastest, slowest) = cycles(&console, l);
            let console_round = fastest - rest(&console);
            let cellgov_round = cycles(&cellgov, l).0 - rest(&cellgov);
            ((cellgov_round - console_round).abs() > slowest - fastest).then(|| {
                format!(
                    "loop {l}: CellGov {cellgov_round:.0} cycles a round beside the \
                     console's {console_round:.0}, spread {:.0}",
                    slowest - fastest
                )
            })
        })
        .collect();
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}
