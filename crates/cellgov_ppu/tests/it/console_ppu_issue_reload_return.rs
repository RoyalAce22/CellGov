//! CellGov's PPU timing against the `ppu_issue_reload_return` console
//! capture. Each loop loads four lines B0 to B3 of one L1 set in order,
//! which makes B0 the set's next victim, and touches a fifth line B4 of
//! the set in memory, whose reload evicts a line when it returns:
//!
//! - loops 0 to 6: B4's touch, then 0, 6, 7, 8, 12, 16 or 32 touches of
//!   memory lines in other sets, then a load of B0, then a wait that lets
//!   the touches go;
//! - loop 7: B4's touch, a wait past its return, then a load of B0;
//! - loop 8: loop 7 with a load of B0 before the wait;
//! - loop 9: loop 7 with no touch.
//!
//! The `console_ppu_issue_touch_turn` tests assert what each touch of
//! loops 1 to 6 adds to the pass. The tests do not assert the cycles of
//! loops 0 to 8. A load of B0 right after B4's touch makes the console's
//! pass about 76 cycles longer (loop 8 against loop 7), and CellGov's
//! pass 1 cycle longer.

use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_reload_return/ps3/cech20-cex-493/observation.json";

/// The payload's word 23 when the probe wrote every word.
const DONE: u64 = 0x5050_5252;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 12;

/// `ld r11, k(r12)`: B0 to B3, 8 KB apart in one L1 set.
const LD_B: [u32; 4] = [0xe96c_0000, 0xe96c_2000, 0xe96c_4000, 0xe96c_6000];
/// `add r12, r10, r11`: B0 again, after the load returns.
const ADD_B0: u32 = 0x7d8a_5a14;
/// `dcbt r24, r25`: B4.
const DCBT_B4: u32 = 0x7c18_ca2c;
/// `addis r25, r25, 1`.
const NEXT_B4: u32 = 0x3f39_0001;
/// `clrlwi r25, r25, 9`.
const WRAP_B4: u32 = 0x5739_027e;
/// `dcbt r28, r27`: a memory line in another set.
const DCBT_OTHER: u32 = 0x7c1c_da2c;
/// `addi r27, r27, 256`.
const NEXT_OTHER: u32 = 0x3b7b_0100;
/// `clrlwi r27, r27, 10`.
const WRAP_OTHER: u32 = 0x577b_02be;
/// `mulld r12, r12, r15`, with r15 = 1.
const MULLD: u32 = 0x7d8c_79d2;
/// `ori 0, 0, 0`.
const NOP: u32 = 0x6000_0000;

/// Where the replay places the probe's three buffers.
const CONFLICT: u64 = 0x80_0000;
const FAR: u64 = 0x100_0000;
const OTHERS: u64 = 0x200_0000;
/// The offset of the L1 set, 37, in a line address.
const SET_OFF: u64 = 37 * 128;
/// B0's address.
const B0: u64 = CONFLICT + SET_OFF;

const SEEDS: &[Seed] = &[
    Seed::Gpr(10, B0),
    Seed::Gpr(12, B0),
    Seed::Gpr(15, 1),
    Seed::Gpr(24, FAR + 0x8000 + SET_OFF),
    Seed::Gpr(25, 0),
    Seed::Gpr(27, 0),
    Seed::Gpr(28, OTHERS),
    // Sizes the replay's memory past the buffer of other lines.
    Seed::Ring(30, OTHERS, 0x50_0000, false),
];

/// The other touches of loops 0 to 6.
pub(crate) const OTHER_TOUCHES: [u64; 7] = [0, 6, 7, 8, 12, 16, 32];

/// The console's payload as big-endian words.
pub(crate) fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 192);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[23], DONE, "the probe wrote every word");
    assert_eq!((words[21], words[22]), (5, u64::from(ITERATIONS_LOG2)));
    assert_eq!(words[20], 0, "every buffer is 64 KB aligned");
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
pub(crate) fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    words.push(0x4200_0000 | ((4 * body.len() as u32).wrapping_neg() & 0xfffc));
    Box::leak(words.into_boxed_slice())
}

/// Loop `k`'s body, in the probe's order.
fn body(k: usize) -> Vec<u32> {
    let chain: Vec<u32> = LD_B.iter().flat_map(|&ld| [ld, ADD_B0]).collect();
    let touch = [DCBT_B4, NEXT_B4, WRAP_B4];
    let load = [LD_B[0], ADD_B0];
    let wait = |n: usize| vec![MULLD; n];
    match k {
        0..=6 => {
            let n = OTHER_TOUCHES[k];
            let others: Vec<u32> = (0..n)
                .flat_map(|_| [DCBT_OTHER, NEXT_OTHER])
                .chain([WRAP_OTHER])
                .collect();
            [
                &chain[..],
                &touch,
                &others,
                &load,
                &wait(5 * n as usize + 41),
            ]
            .concat()
        }
        7 => [&chain[..], &touch, &[NOP, NOP], &wait(56), &load].concat(),
        8 => [&chain[..], &touch, &load, &wait(56), &load].concat(),
        _ => [&chain[..], &[NOP; 5], &wait(56), &load].concat(),
    }
}

/// CellGov's cycles an iteration for loop `k`.
pub(crate) fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&body(k)),
        seeds: SEEDS,
    };
    replay(&r, 0).expect("the loop replays")
}

/// The loop that loads B0 only after B4's reload returns.
const AFTER_RETURN: usize = 7;
/// The loop that also hits B0 before B4's reload returns.
const HIT_BEFORE_RETURN: usize = 8;

#[test]
fn a_hit_on_the_victim_before_the_touch_reload_returns_makes_no_later_load_faster() {
    let w = console_words();
    let (after, hit) = (
        cellgov_cycles(AFTER_RETURN),
        cellgov_cycles(HIT_BEFORE_RETURN),
    );
    let (console_after, console_hit) = (
        console_cycles(&w, AFTER_RETURN),
        console_cycles(&w, HIT_BEFORE_RETURN),
    );
    for (who, hit, after) in [
        ("CellGov", hit, after),
        ("console", console_hit, console_after),
    ] {
        assert!(
            hit >= after,
            "{who}: with a hit on B0 before B4's reload returns, the loop takes {hit:.2} \
             cycles, less than without it ({after:.2})",
        );
    }
}
