//! CellGov's PPU timing against the `ppu_issue_page_cross_alias`,
//! `ppu_issue_page_cross_store` and `ppu_issue_page_cross_waits` console
//! captures.
//!
//! Each probe maps one segment at X and at Y = X + 1 MiB. The page after
//! X's last page is then Y's first page, which maps to the segment's
//! first real page. An `ld` or `std` at the last 4 bytes of X reaches
//! its second line at that real page, which X + 0 also reaches.

use cellgov_core::Runtime;
use cellgov_exec::UnitStatus;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::mmu::Partition;
use cellgov_ppu::supervisor::{SlbEntry, SupervisorState};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

/// The page table: 2^11 groups at 256 KB, ending at the memory's end.
const HTAB: u64 = 0x4_0000;
const MEMORY: u64 = 0x8_0000;
/// The segment SLB entry 0 maps.
const SEGMENT: u64 = 0x1000_0000;
const VSID: u64 = 0x123;
/// Page 0 of the segment holds the loop, at real 0x3000.
const CODE_RA: u64 = 0x3000;
/// The two effective addresses of the shared storage.
const X: u64 = SEGMENT + 0x10_0000;
const Y: u64 = X + 0x10_0000;
/// The real page of the storage's first page, which X and Y reach.
const FIRST_RA: u64 = 0x1_0000;
/// The real page of the storage's last page, which X reaches. The real
/// page after it is not [`FIRST_RA`].
const LAST_RA: u64 = 0x1_1000;
/// The start of X's last line. The doubleword at `P + 0x7C` crosses
/// into Y.
const P: u64 = X + 0xF_FF80;

/// `dcbf 0,r8; sync; ld r9,0x7C(r9); bdnz`.
const FLUSH_LOAD: &[u32] = &[0x7C00_40AC, 0x7C00_04AC, 0xE929_007C, 0x4200_FFF4];
/// `sync; ld r9,0x7C(r9); bdnz`.
const SYNC_LOAD: &[u32] = &[0x7C00_04AC, 0xE929_007C, 0x4200_FFF8];
/// `dcbf 0,r8; sync; std r9,0x7C(r9); sync; ld r10,0(r8); bdnz`.
const FLUSH_CROSS_LOAD: &[u32] = &[
    0x7C00_40AC,
    0x7C00_04AC,
    0xF929_007C,
    0x7C00_04AC,
    0xE948_0000,
    0x4200_FFEC,
];
/// `dcbf 0,r8; sync; std r9,0x7C(r9); sync; bdnz`.
const FLUSH_CROSS: &[u32] = &[
    0x7C00_40AC,
    0x7C00_04AC,
    0xF929_007C,
    0x7C00_04AC,
    0x4200_FFF0,
];
/// `dcbf 0,r8; sync; stw r10,0(r11); sync; bdnz`.
const FLUSH_PLAIN: &[u32] = &[
    0x7C00_40AC,
    0x7C00_04AC,
    0x914B_0000,
    0x7C00_04AC,
    0x4200_FFF0,
];
/// `dcbf 0,r8; sync; stw r10,0(r11); sync; ld r12,0(r8); bdnz`.
const FLUSH_PLAIN_LOAD: &[u32] = &[
    0x7C00_40AC,
    0x7C00_04AC,
    0x914B_0000,
    0x7C00_04AC,
    0xE988_0000,
    0x4200_FFEC,
];
/// `dcbf 0,r8; ld r9,0x7C(r9); sync; bdnz`.
const FLUSH_THEN_LOAD: &[u32] = &[0x7C00_40AC, 0xE929_007C, 0x7C00_04AC, 0x4200_FFF4];
/// `dcbf 0,r8; sync; ld r9,0x7C(r9); sync; bdnz`.
const FLUSH_SYNC_LOAD: &[u32] = &[
    0x7C00_40AC,
    0x7C00_04AC,
    0xE929_007C,
    0x7C00_04AC,
    0x4200_FFF0,
];
/// `dcbf 0,r8; sync; dcbt 0,r8; ld r9,0x7C(r9); sync; bdnz`.
const FLUSH_TOUCH_SAME_LOAD: &[u32] = &[
    0x7C00_40AC,
    0x7C00_04AC,
    0x7C00_422C,
    0xE929_007C,
    0x7C00_04AC,
    0x4200_FFEC,
];
/// `dcbf 0,r8; sync; dcbt 0,r7; ld r9,0x7C(r9); sync; bdnz`.
const FLUSH_TOUCH_OTHER_LOAD: &[u32] = &[
    0x7C00_40AC,
    0x7C00_04AC,
    0x7C00_3A2C,
    0xE929_007C,
    0x7C00_04AC,
    0x4200_FFEC,
];
/// The line r7 holds, which no load in the probes reaches.
const OTHER: u64 = X + 0x100;

/// One probe's capture and the shape of its payload.
struct Probe {
    capture: &'static str,
    /// The payload's words.
    words: usize,
    /// Each case's loop and flushed line, in the probe's case order.
    cases: &'static [(&'static [u32], u64)],
    /// The first of the four teardown words.
    teardown: usize,
    /// The word after the teardown words: 1 when the probe's storage
    /// checks held.
    held: usize,
    /// The last word's value.
    mark: u64,
}

const ALIAS: Probe = Probe {
    capture: "../../tests/micro/ppu_issue_page_cross_alias/ps3/cech20-cex-493/observation.json",
    words: 26,
    cases: &[
        (FLUSH_LOAD, X),
        (FLUSH_LOAD, Y),
        (FLUSH_LOAD, X + 0x80),
        (FLUSH_LOAD, P),
        (SYNC_LOAD, X + 0x80),
        (FLUSH_CROSS_LOAD, X),
        (FLUSH_CROSS_LOAD, X + 0x80),
    ],
    teardown: 20,
    held: 24,
    mark: 0x5050_4341,
};

const STORE: Probe = Probe {
    capture: "../../tests/micro/ppu_issue_page_cross_store/ps3/cech20-cex-493/observation.json",
    words: 24,
    cases: &[
        (FLUSH_CROSS, X),
        (FLUSH_CROSS, X + 0x80),
        (FLUSH_PLAIN, X),
        (FLUSH_PLAIN, X + 0x80),
        (FLUSH_PLAIN_LOAD, X),
        (FLUSH_PLAIN_LOAD, X + 0x80),
    ],
    teardown: 18,
    held: 22,
    mark: 0x5050_4353,
};

const WAITS: Probe = Probe {
    capture: "../../tests/micro/ppu_issue_page_cross_waits/ps3/cech20-cex-493/observation.json",
    words: 20,
    cases: &[
        (FLUSH_THEN_LOAD, X),
        (FLUSH_SYNC_LOAD, X),
        (FLUSH_TOUCH_SAME_LOAD, X),
        (FLUSH_TOUCH_OTHER_LOAD, X),
    ],
    teardown: 14,
    held: 18,
    mark: 0x5050_4357,
};

/// The iterations of a case's replay in each half.
const ITERATIONS: u64 = 64;

/// Each case's smallest cycles an iteration on the console.
fn console(probe: &Probe) -> Vec<f64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(probe.capture);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 4 * probe.words);
    let word = |i: usize| {
        u64::from(u32::from_be_bytes(
            p[4 * i..4 * i + 4].try_into().expect("4"),
        ))
    };
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");
    assert_eq!(word(5), word(4) + 0x10_0000, "Y follows X");
    for w in probe.teardown..probe.teardown + 4 {
        assert_eq!(word(w), 0, "teardown word {w} returned 0");
    }
    assert_eq!(word(probe.held), 1, "the probe's storage checks held");
    assert_eq!(
        word(probe.words - 1),
        probe.mark,
        "the probe reached its teardown"
    );
    let iterations = word(3);
    (0..probe.cases.len())
        .map(|c| (word(6 + 2 * c) * PPU_CYCLES_PER_TB_TICK) as f64 / iterations as f64)
        .collect()
}

fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).expect("range");
    mem.apply_commit(range, bytes).expect("mapped");
}

/// Maps the page at `ea` to `ra` in slot 0 of its primary group, read and
/// write for key 0. Each page index here is below 2^11, so the AVPN is
/// the VSID.
fn map_page(mem: &mut GuestMemory, ea: u64, ra: u64) {
    let index = (ea - SEGMENT) / 0x1000;
    let pteg = HTAB + ((VSID ^ index) & 0x7FF) * 128;
    put(mem, pteg, &((VSID << 12) | 1).to_be_bytes());
    put(mem, pteg + 8, &(ra | 0b10).to_be_bytes());
}

/// The cycles an iteration of `words` takes on a thread that translates,
/// with r7 at [`OTHER`], r8 at `flushed`, r9 and r10 at P and r11 at X.
fn cycles_an_iteration(words: &[u32], flushed: u64) -> f64 {
    let mut mem = GuestMemory::new(MEMORY as usize);
    map_page(&mut mem, SEGMENT, CODE_RA);
    map_page(&mut mem, X, FIRST_RA);
    map_page(&mut mem, X + 0xF_F000, LAST_RA);
    map_page(&mut mem, Y, FIRST_RA);
    for (i, w) in words.iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &w.to_be_bytes());
    }
    // The storage's first word holds P; its last word is 0.
    put(&mut mem, FIRST_RA, &(P as u32).to_be_bytes());
    let exit = SEGMENT + 4 * words.len() as u64;

    let mut rt = Runtime::new(mem, Budget::new(1), usize::MAX);
    let unit = rt.register_unit_with(|id| {
        let mut sup = SupervisorState::new(MSR_SF | MSR_IR | MSR_DR);
        sup.slb[0] = SlbEntry {
            esid_v: SEGMENT | SLB_VALID,
            vsid_flags: VSID << 12,
        };
        sup.partition =
            Partition::new(HTAB, 1 << 62, 0, Partition::INITIAL.lb()).expect("partition");
        let mut ppu = PpuExecutionUnit::new(id);
        let state = ppu.state_mut();
        state.set_supervisor(Some(sup));
        state.pc = SEGMENT;
        state.set_ctr(2 * ITERATIONS);
        state.set_gpr(8, flushed);
        state.set_gpr(9, P);
        state.set_gpr(10, P);
        state.set_gpr(11, X);
        state.set_gpr(7, OTHER);
        ppu
    });
    let state = |rt: &Runtime| -> cellgov_ppu::state::PpuState {
        rt.registry()
            .get(unit)
            .and_then(|u| u.as_any().downcast_ref::<PpuExecutionUnit>())
            .expect("the PPU unit")
            .state()
            .clone()
    };
    let mut halfway = None;
    for _ in 0..4 * 2 * ITERATIONS * words.len() as u64 {
        let s = state(&rt);
        if s.pc == exit {
            break;
        }
        if halfway.is_none() && s.pc == SEGMENT && s.ctr() == ITERATIONS {
            halfway = Some(rt.registry().clock(unit).raw());
        }
        assert_ne!(
            rt.registry().effective_status(unit),
            Some(UnitStatus::Faulted),
            "the loop faulted at pc {:#x}",
            s.pc
        );
        let mut step = rt.step().expect("the step runs");
        rt.commit_step(&mut step).expect("the step commits");
    }
    let s = state(&rt);
    assert_eq!(s.pc, exit, "the loop ran to its exit");
    assert_eq!(s.gpr[9], P, "the loop kept P across the page");
    let halfway = halfway.expect("the loop passed its halfway count");
    (rt.registry().clock(unit).raw() - halfway) as f64 / ITERATIONS as f64
}

fn cellgov(probe: &Probe) -> Vec<f64> {
    probe
        .cases
        .iter()
        .map(|&(words, flushed)| cycles_an_iteration(words, flushed))
        .collect()
}

/// The largest share of the console's cycles that the model may differ by.
const BAND: f64 = 0.20;

/// The cases the model holds within [`BAND`] of the console, by probe.
///
/// In `ppu_issue_page_cross_alias` case 5 and `ppu_issue_page_cross_store`
/// cases 0, 2 and 4, a store finds its line missing from the L2. The
/// next `sync` then waits for the line's fetch from memory. The wait is
/// the same for a crossing `std` and a plain `stw`.
const HELD: [(&Probe, &[usize]); 3] = [
    (&ALIAS, &[0, 1, 3, 4, 5, 6]),
    (&STORE, &[0, 2, 3, 4, 5]),
    (&WAITS, &[0, 1, 2, 3]),
];

/// The cases the model does not hold, by probe. The model gives each
/// fewer cycles than the console:
///
/// - `ppu_issue_page_cross_alias` case 2: a `dcbf` of a line the caches
///   do not hold, then `sync`, costs the console about 110 cycles more
///   than the `sync` alone (case 4). The model charges nothing for it.
/// - `ppu_issue_page_cross_store` case 1: the same `dcbf` of a line the
///   caches do not hold comes before a crossing `std` to lines the L2
///   holds. The console takes 52 cycles an iteration more than for a
///   plain `stw` there (case 3). The model gives the two the same cycles.
const DIFFERENT: [(&Probe, &[usize]); 2] = [(&ALIAS, &[2]), (&STORE, &[1])];

#[test]
fn each_held_case_comes_within_the_band_of_the_console() {
    for (probe, cases) in HELD {
        let (console, cellgov) = (console(probe), cellgov(probe));
        for &c in cases {
            assert!(
                (cellgov[c] - console[c]).abs() <= BAND * console[c],
                "{} case {c}: CellGov {:.2} cycles an iteration, the console {:.2}",
                probe.capture,
                cellgov[c],
                console[c],
            );
        }
    }
}

/// If the model comes within [`BAND`] of a case, move the case into
/// [`HELD`].
#[test]
fn each_other_case_is_a_known_difference_under_the_console() {
    for (probe, cases) in DIFFERENT {
        let (console, cellgov) = (console(probe), cellgov(probe));
        for &c in cases {
            assert!(
                cellgov[c] < (1.0 - BAND) * console[c],
                "{} case {c}: CellGov {:.2} cycles an iteration, the console {:.2}",
                probe.capture,
                cellgov[c],
                console[c],
            );
        }
    }
}

#[test]
fn a_crossing_load_waits_for_its_next_pages_real_line_flushed_through_either_address() {
    for (who, c) in [("console", console(&ALIAS)), ("CellGov", cellgov(&ALIAS))] {
        assert!(
            (c[0] - c[1]).abs() < c[0] / 20.0,
            "{who}: a flush through X ({:.2} cycles) and through Y ({:.2}) cost the load alike",
            c[0],
            c[1],
        );
        assert!(
            (c[0] - c[3]).abs() < c[3] / 10.0,
            "{who}: the load waits for its second line ({:.2} cycles) as for its first ({:.2})",
            c[0],
            c[3],
        );
        assert!(
            c[2] < c[0] / 1.5,
            "{who}: a flush of a line the load does not reach ({:.2} cycles) leaves it under \
             a flush of its second line ({:.2})",
            c[2],
            c[0],
        );
    }
}

/// The load after the store costs what case 5 or 6 of
/// `ppu_issue_page_cross_alias` adds to case 0 or 1 of
/// `ppu_issue_page_cross_store`, which run the same loop without it.
#[test]
fn a_load_after_a_crossing_store_finds_the_stores_second_line_near() {
    for (who, alias, store) in [
        ("console", console(&ALIAS), console(&STORE)),
        ("CellGov", cellgov(&ALIAS), cellgov(&STORE)),
    ] {
        let second_line = alias[5] - store[0];
        let memory = alias[6] - store[1];
        assert!(
            second_line < memory / 3.0,
            "{who}: a load of the store's second line takes {second_line:.2} cycles, a load \
             from memory {memory:.2}",
        );
    }
}
