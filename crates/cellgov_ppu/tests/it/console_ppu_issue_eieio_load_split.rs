//! CellGov's PPU timing against the `ppu_issue_eieio_load_split` console
//! capture. Each loop flushes the store's line or the next line, then
//! runs a `stw`, one of these bodies and 256 dependent `addi`:
//!
//! - no barrier, an `eieio` or an `lwsync`, with or without a load of a
//!   held line after it;
//! - a load of a held line, then an `eieio`.

use crate::console_ppu_issue_store_miss_eieio::{
    cellgov_cycles, console_cycles, EIEIO, LWSYNC, LWZ, STW, X,
};
use cellgov_ps3_abi::hw::ppu_issue::PPU_STORE_MISS_FETCH_CYCLES;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_eieio_load_split/ps3/cech20-cex-493/observation.json";

/// The payload's words.
const WORDS: usize = 34;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5045_4C53;

/// The cases in the probe's order: the body after `dcbf 0,r3; sync`, and
/// the line the loop flushes.
const CASES: [(&[u32], u64); 14] = [
    (&[STW], X),
    (&[STW], X + 0x80),
    (&[STW, EIEIO], X),
    (&[STW, EIEIO], X + 0x80),
    (&[STW, LWZ], X),
    (&[STW, LWZ], X + 0x80),
    (&[STW, EIEIO, LWZ], X),
    (&[STW, EIEIO, LWZ], X + 0x80),
    (&[STW, LWSYNC], X),
    (&[STW, LWSYNC], X + 0x80),
    (&[STW, LWSYNC, LWZ], X),
    (&[STW, LWSYNC, LWZ], X + 0x80),
    (&[STW, LWZ, EIEIO], X),
    (&[STW, LWZ, EIEIO], X + 0x80),
];

fn console() -> Vec<f64> {
    console_cycles(CONSOLE, WORDS, DONE, CASES.len())
}

fn cellgov() -> Vec<f64> {
    cellgov_cycles(&CASES)
}
#[test]
fn an_eieio_after_a_store_miss_adds_less_than_half_the_lines_trip() {
    let fetch = PPU_STORE_MISS_FETCH_CYCLES as f64;
    let pairs = [
        (2, 0, "before no load"),
        (6, 4, "before the load"),
        (12, 4, "after the load"),
    ];
    for (who, cycles) in [("console", console()), ("CellGov", cellgov())] {
        for (with, without, place) in pairs {
            let adds = cycles[with] - cycles[without];
            assert!(
                adds < fetch / 2.0,
                "{who}: an eieio {place} adds {adds:.0} cycles to a store miss's loop",
            );
        }
    }
}

/// The largest share of the console's cycles that the model may differ by.
const BAND: f64 = 0.20;

#[test]
fn each_case_comes_within_the_band_of_the_console() {
    let (console, cellgov) = (console(), cellgov());
    for c in 0..CASES.len() {
        assert!(
            (cellgov[c] - console[c]).abs() <= BAND * console[c],
            "case {c}: CellGov {:.2} cycles an iteration, the console {:.2}",
            cellgov[c],
            console[c],
        );
    }
}
