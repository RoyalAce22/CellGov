//! CellGov's `sys_memory_allocate` round trip against the
//! `ppu_lv2_allocate_flags` console capture: allocations of 1 and 16 MiB
//! whose flags name no page size, and the same sizes with the 1 MiB page
//! flag. The probe times each call alone, 16 rounds.
//!
//! In this console capture, the 16 MiB allocation with the 1 MiB page
//! flag takes 4.1% more than the line through the `ppu_lv2_call_sizes`
//! console capture gives. The same call moves that far between two runs
//! on the console.

use cellgov_ps3_abi::hw::ppu_issue::{
    ppu_syscall_success_round_trip_cycles, PPU_CYCLES_PER_TB_TICK, PPU_SYSCALL_ROUND_TRIP_CYCLES,
};
use cellgov_ps3_abi::lv2::errno::CELL_ENOMEM;
use cellgov_ps3_abi::lv2::memory::page_size;
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str =
    "../../tests/micro/ppu_lv2_allocate_flags/ps3/cech20-cex-493/observation.json";

/// The payload's word 60 when the probe wrote every word.
const DONE: u64 = 0x414C_4C46;

/// Rounds the probe makes.
const ROUNDS: usize = 16;

/// Each allocation the probe makes, as `(size, flags)`. The last one sets
/// bit 32 of a 1 MiB size.
const ALLOCATIONS: [(u64, u64); 5] = [
    (1 << 20, page_size::FLAG_1M),
    (16 << 20, page_size::FLAG_1M),
    (1 << 20, 0),
    (16 << 20, 0),
    ((1 << 32) | (1 << 20), page_size::FLAG_1M),
];

/// Calls a round makes.
const CALLS: usize = 1 + 2 * ALLOCATIONS.len();

/// The allocation whose size does not fit in 32 bits.
const WIDE: usize = 4;

/// Each allocation's lower median sample, in core cycles, less the
/// cycles the bracket adds beside the cheapest call's round trip.
///
/// The console refuses the wide allocation in every round, and its first
/// refusal is `CELL_ENOMEM`. Every other allocation and every free
/// returns 0 in every round.
fn console_allocation_trips() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 512 + 4 * CALLS * ROUNDS);
    let w: Vec<u64> = p[..512]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[60], DONE, "the probe wrote every word");
    assert_eq!(w[56], ROUNDS as u64, "rounds");
    assert_eq!(w[57], CALLS as u64, "calls a round");
    for a in 0..ALLOCATIONS.len() {
        let call = 1 + 2 * a;
        let (rc, done) = if a == WIDE {
            (u64::from(CELL_ENOMEM.code), 0)
        } else {
            (0, ROUNDS as u64)
        };
        assert_eq!(w[call] as u32, rc as u32, "allocation {a}'s return");
        assert_eq!(w[call + 1], 0, "every free after allocation {a} returned 0");
        assert_eq!(w[40 + a], done, "allocation {a}'s rounds that returned 0");
    }
    assert_eq!(w[48], w[1 + 2 * WIDE], "the wide allocation's first answer");
    let medians: Vec<u64> = p[512..]
        .chunks_exact(4 * ROUNDS)
        .map(|call| {
            let mut s: Vec<u64> = call
                .chunks_exact(4)
                .map(|s| u64::from(u32::from_be_bytes(s.try_into().expect("4 bytes"))))
                .collect();
            s.sort_unstable();
            s[(s.len() - 1) / 2] * PPU_CYCLES_PER_TB_TICK
        })
        .collect();
    let rest = medians[0] - PPU_SYSCALL_ROUND_TRIP_CYCLES;
    (0..ALLOCATIONS.len())
        .map(|a| medians[1 + 2 * a] - rest)
        .collect()
}

#[test]
fn an_allocation_whose_flags_name_no_page_size_takes_the_1m_page_round_trip() {
    let trips = console_allocation_trips();
    for (default, flagged) in [(2, 0), (3, 1)] {
        let (size, flags) = ALLOCATIONS[default];
        assert_eq!(flags, 0);
        assert_eq!(ALLOCATIONS[flagged], (size, page_size::FLAG_1M));
        let console = trips[default] as f64;
        let console_flagged = trips[flagged] as f64;
        assert!(
            (console - console_flagged).abs() <= 0.04 * console_flagged,
            "{size:#x} bytes: the console takes {console:.0} with no page flag \
             and {console_flagged:.0} with the 1 MiB flag",
        );
        let model = ppu_syscall_success_round_trip_cycles(sc::MEMORY_ALLOCATE, size, flags);
        assert_eq!(
            model,
            ppu_syscall_success_round_trip_cycles(sc::MEMORY_ALLOCATE, size, page_size::FLAG_1M),
            "{size:#x} bytes: CellGov times no page flag as 1 MiB pages",
        );
        let model = model as f64;
        assert!(
            (model - console).abs() <= 0.04 * console,
            "{size:#x} bytes with no page flag: CellGov {model:.0}, the console {console:.0}",
        );
    }
}
