//! CellGov's PPU timing against the `ppu_issue_vscr_mem` console capture:
//! whether an `mtvscr` stalls the VMX and FP loads and stores after it.
//!
//! The probe times twelve loops with `mftb`, five runs each. Every load
//! and store addresses one data line, which r8 holds.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_vscr_mem/ps3/cech20-cex-493/observation.json";

/// The payload's word 26 when the probe wrote every word.
const DONE: u64 = 0x5050_564d;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 18;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 216);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[26], DONE, "the probe wrote every word");
    assert_eq!((words[24], words[25]), (5, u64::from(ITERATIONS_LOG2)));
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    words.push(0x4200_0000 | ((4 * body.len() as u32).wrapping_neg() & 0xfffc));
    Box::leak(words.into_boxed_slice())
}

/// `vaddubm vk, v20, v21`.
const fn vadd(k: u32) -> u32 {
    0x1014_a800 | (k << 21)
}

/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;
/// `lvx v1, 0, r8`.
const LVX: u32 = 0x7c20_40ce;
/// `stvx v20, 0, r8`.
const STVX: u32 = 0x7e80_41ce;
/// `lfd f1, 0(r8)`.
const LFD: u32 = 0xc828_0000;
/// `stfd f2, 0(r8)`.
const STFD: u32 = 0xd848_0000;
/// `vaddubm v2, v1, v1`.
const VADD_USE: u32 = 0x1041_0800;
/// `addi r14..r17, r14..r17, 1`.
const ADDI: [u32; 4] = [0x39ce_0001, 0x39ef_0001, 0x3a10_0001, 0x3a31_0001];

/// Each loop's body, in the probe's order.
fn bodies() -> [Vec<u32>; 12] {
    let vadd4: Vec<u32> = (1..=4).map(vadd).collect();
    [
        vec![MTVSCR],
        vec![LVX],
        vec![MTVSCR, LVX],
        vec![STVX],
        vec![MTVSCR, STVX],
        vec![LFD],
        vec![MTVSCR, LFD],
        vec![STFD],
        vec![MTVSCR, STFD],
        [&[MTVSCR][..], &vadd4].concat(),
        [&[MTVSCR, LVX][..], &ADDI].concat(),
        vec![MTVSCR, LVX, VADD_USE],
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize, body: &[u32]) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(body),
        seeds: &[Seed::Gpr(8, ONE_LINE)],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The loops of an `mtvscr` and then an `lvx`, an `stvx` or an `lfd`.
const FENCED_ACCESSES: [usize; 3] = [2, 4, 6];

/// The largest share of a fenced access loop's fastest console run the
/// model may fall short by: the `stvx` loop falls 2.6% short.
const GAP: f64 = 0.04;

#[test]
fn an_mtvscr_holds_each_vmx_load_and_store_and_an_fp_load_after_it_within_the_gap() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64)> = FENCED_ACCESSES
        .iter()
        .map(|&k| (k, cellgov_cycles(k, &bodies()[k]), console_cycles(&w, k)))
        .collect();
    for &(k, cellgov, fastest) in &rows {
        assert!(
            cellgov <= fastest && (fastest - cellgov) / fastest <= GAP,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {fastest:.2}, more than {GAP} of it apart or above it: {rows:?}",
        );
    }
}

/// Two loops the model does not time to the console:
///
/// - an `stfd` after an `mtvscr` costs the console about twelve cycles
///   more than the other fenced accesses;
/// - four `addi` after the fenced `lvx` cost the model about three cycles
///   more than the console.
///
/// The test fails when the model drops the fence for the `stfd` or when
/// either difference closes. Then move that loop into `FENCED_ACCESSES`.
#[test]
fn the_fenced_stfd_and_the_addi_after_the_fenced_lvx_are_known_differences() {
    let w = console_words();
    let lvx = console_cycles(&w, 2);
    let stfd = (cellgov_cycles(8, &bodies()[8]), console_cycles(&w, 8));
    let addi = (cellgov_cycles(10, &bodies()[10]), console_cycles(&w, 10));
    assert!(
        stfd.0 <= lvx && (lvx - stfd.0) / lvx <= GAP,
        "loop 8: CellGov {:.2}, not held at the fence the console's fenced lvx loop \
         shows ({lvx:.2})",
        stfd.0
    );
    assert!(
        (stfd.1 - stfd.0) / stfd.1 > 0.10,
        "loop 8: CellGov {:.2}, the console's fastest {:.2}",
        stfd.0,
        stfd.1
    );
    assert!(
        addi.0 > addi.1,
        "loop 10: CellGov {:.2}, the console's fastest {:.2}",
        addi.0,
        addi.1
    );
}
