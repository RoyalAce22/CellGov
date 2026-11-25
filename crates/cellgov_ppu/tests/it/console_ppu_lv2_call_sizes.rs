//! CellGov's LV2 round trips against the `ppu_lv2_call_sizes` console
//! capture: the PRX module calls on five modules, and the memory allocate
//! and free calls at six sizes. The probe times each call alone, 16 rounds.
//!
//! Each call sits between two `mftb` reads, as in the
//! `ppu_lv2_call_heavy` console capture. The cheapest call's samples in
//! the same bracket give the cycles the bracket adds beside its round
//! trip.

use cellgov_ps3_abi::hw::ppu_issue::{
    ppu_syscall_round_trip_cycles, ppu_syscall_success_round_trip_cycles, PPU_CYCLES_PER_TB_TICK,
    PPU_SYSCALL_ROUND_TRIP_CYCLES,
};
use cellgov_ps3_abi::lv2::memory::page_size;
use cellgov_ps3_abi::lv2::syscall as sc;

const CONSOLE: &str = "../../tests/micro/ppu_lv2_call_sizes/ps3/cech20-cex-493/observation.json";

/// The payload's word 60 when the probe wrote every word.
const DONE: u64 = 0x5050_4C5A;

/// Rounds the probe makes.
const ROUNDS: usize = 16;

/// Modules the probe loads.
const MODULES: usize = 5;

/// Calls a round makes.
const CALLS: usize = 1 + 6 * MODULES + 2 * ALLOCATIONS.len();

/// The index in a round of the first allocation call.
const ALLOCATION_CALL: usize = 1 + 6 * MODULES;

/// Each allocation the probe makes, as `(size, flags)`.
const ALLOCATIONS: [(u64, u64); 6] = [
    (1 << 20, page_size::FLAG_1M),
    (4 << 20, page_size::FLAG_1M),
    (16 << 20, page_size::FLAG_1M),
    (64 << 10, page_size::FLAG_64K),
    (1 << 20, page_size::FLAG_64K),
    (4 << 20, page_size::FLAG_64K),
];

/// Each call's lower median sample, in core cycles, less the cycles the
/// bracket adds beside the cheapest call's round trip.
fn console_trips() -> Vec<u64> {
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
    for (c, &rc) in w.iter().enumerate().take(CALLS).skip(1) {
        assert_eq!(rc, 0, "every call {c} returned 0, or an id for a load");
    }
    for m in 0..MODULES {
        assert_eq!(
            w[48 + m],
            ROUNDS as u64,
            "every load of module {m} returned an id"
        );
    }
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
    assert_eq!(rest, 2431, "the bracket's own cycles");
    medians.iter().map(|m| m - rest).collect()
}

#[test]
fn each_allocation_takes_within_four_percent_of_its_size_s_round_trip() {
    let trips = console_trips();
    for (a, &(size, flags)) in ALLOCATIONS.iter().enumerate() {
        let console = trips[ALLOCATION_CALL + 2 * a] as f64;
        let model = ppu_syscall_success_round_trip_cycles(sc::MEMORY_ALLOCATE, size, flags) as f64;
        assert!(
            (model - console).abs() <= 0.04 * console,
            "{size:#x} bytes under flags {flags:#x}: CellGov {model:.0}, the console {console:.0}",
        );
    }
}

#[test]
fn a_module_call_does_not_follow_the_module_s_size() {
    let trips = console_trips();
    let phase = |p: usize| -> Vec<u64> { (0..MODULES).map(|m| trips[1 + 6 * m + p]).collect() };
    let span = |v: &[u64]| {
        (
            *v.iter().min().expect("five modules"),
            *v.iter().max().expect("five modules"),
        )
    };
    let loads = phase(0);
    assert_eq!(span(&loads), (67_172_649, 248_357_809));
    assert_eq!(loads[0], 116_098_049, "libfiber.sprx, the smallest module");
    assert!(
        loads[0] > loads[1],
        "libfiber.sprx loads slower than libadec.sprx"
    );
    let heavy = ppu_syscall_round_trip_cycles(sc::SYS_PRX_LOAD_MODULE) as f64;
    let ratio = loads[1] as f64 / heavy;
    assert!(
        (2.4..2.6).contains(&ratio),
        "libadec.sprx's loads: {ratio:.2}"
    );
    assert_eq!(span(&phase(1)), (284_209, 1_749_529), "a first start phase");
    assert_eq!(span(&phase(4)), (189_049, 1_238_849), "a second stop phase");
}

#[test]
fn a_free_grows_with_its_pages_and_the_table_keeps_the_1_mib_block_s_trip() {
    let trips = console_trips();
    let frees: Vec<u64> = (0..ALLOCATIONS.len())
        .map(|a| trips[ALLOCATION_CALL + 2 * a + 1])
        .collect();
    assert_eq!(frees.len(), 6);
    assert_eq!(
        (
            *frees.iter().min().expect("six frees"),
            *frees.iter().max().expect("six frees")
        ),
        (41_129, 601_929),
        "a free"
    );
    for (pages, blocks) in [("1 MiB", &frees[..3]), ("64 KiB", &frees[3..])] {
        assert!(
            blocks[0] < blocks[1] && blocks[1] < blocks[2],
            "{pages} pages: frees of larger blocks take longer: {blocks:?}"
        );
    }
    let model = ppu_syscall_round_trip_cycles(sc::MEMORY_FREE) as f64;
    let console = frees[0] as f64;
    assert!(
        (model - console).abs() <= 0.04 * console,
        "the 1 MiB block's free: CellGov {model:.0}, the console {console:.0}",
    );
}
