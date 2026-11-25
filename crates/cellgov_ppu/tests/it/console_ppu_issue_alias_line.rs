//! CellGov's PPU timing against the `ppu_issue_alias_line` console
//! capture.
//!
//! The probe maps one segment at X and at Y and runs a dependent `ld`
//! chase over 192 lines of it. One chase loads each line once through X
//! and once through Y in a lap; the other loads each line through X only.
//! 192 lines fit the data cache and 384 do not. The console takes the
//! same time a load in both chases, so a load through the second address
//! of a line the cache holds hits it.

use cellgov_core::Runtime;
use cellgov_exec::UnitStatus;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::mmu::Partition;
use cellgov_ppu::supervisor::{SlbEntry, SupervisorState};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/ppu_issue_alias_line/ps3/cech20-cex-493/observation.json";

/// The probe's payload: 22 words.
const WORDS: usize = 22;

/// The probe's runs of each ring.
const REPS: usize = 5;

/// The lines each ring spans.
const LINES: u64 = 192;
const LINE: u64 = 128;

/// The page table: 2^11 groups at 256 KB, ending at the memory's end.
const HTAB: u64 = 0x4_0000;
const MEMORY: u64 = 0x8_0000;
/// The segment SLB entry 0 maps.
const SEGMENT: u64 = 0x1000_0000;
const VSID: u64 = 0x123;
/// Page 0 of the segment holds the chase loop, at real 0x3000.
const CODE_RA: u64 = 0x3000;
/// The two effective addresses of the shared storage.
const X: u64 = SEGMENT + 0x10_0000;
const Y: u64 = SEGMENT + 0x20_0000;
/// Each ring's offset from X or Y, and the real address of that offset.
const PLAIN: (u64, u64) = (0x1_0000, 0x1_0000);
const ALIAS: (u64, u64) = (0x3_0000, 0x3_0000);

/// The replay's iterations of eight loads in each half: four laps of a
/// ring's 384 elements.
const ITERATIONS: u64 = 4 * 2 * LINES / 8;

/// Each ring's mean cycles a load on the console.
fn console() -> (f64, f64) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 4 * WORDS);
    let word = |i: usize| {
        u64::from(u32::from_be_bytes(
            p[4 * i..4 * i + 4].try_into().expect("4"),
        ))
    };
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");
    assert_eq!(word(2), REPS as u64);
    assert_ne!(word(4), word(5), "the segment is at two addresses");
    for w in 16..20 {
        assert_eq!(word(w), 0, "teardown word {w} returned 0");
    }
    assert_eq!((word(20), word(21)), (1, 1), "each chase closed its ring");
    let loads = word(3);
    let mean = |first: usize| {
        let ticks: u64 = (first..first + REPS).map(word).sum();
        (ticks * PPU_CYCLES_PER_TB_TICK) as f64 / (REPS as u64 * loads) as f64
    };
    (mean(6), mean(11))
}

fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).expect("range");
    mem.apply_commit(range, bytes).expect("mapped");
}

/// Maps page `index` of the segment to `ra` in slot 0 of its primary
/// group, read and write for key 0. Each page index here is below 2^11,
/// so the AVPN is the VSID.
fn map_page(mem: &mut GuestMemory, index: u64, ra: u64) {
    let pteg = HTAB + ((VSID ^ index) & 0x7FF) * 128;
    put(mem, pteg, &((VSID << 12) | 1).to_be_bytes());
    put(mem, pteg + 8, &(ra | 0b10).to_be_bytes());
}

/// Maps the pages of a ring at `(off, ra)` through effective base `base`.
fn map_ring(mem: &mut GuestMemory, base: u64, (off, ra): (u64, u64)) {
    for k in 0..(LINES * LINE).div_ceil(0x1000) {
        map_page(mem, (base + off - SEGMENT) / 0x1000 + k, ra + 0x1000 * k);
    }
}

/// Element `j` of the ring at `off`, its second half through `high` and 8
/// bytes into each line.
fn element(high: u64, off: u64, j: u64) -> u64 {
    let j = j % (2 * LINES);
    if j < LINES {
        X + off + LINE * j
    } else {
        high + off + LINE * (j - LINES) + 8
    }
}

/// The cycles a load of the ring at `ring` takes, its second half through
/// `high`, on a thread that translates.
fn cycles_a_load(high: u64, ring: (u64, u64)) -> f64 {
    let mut mem = GuestMemory::new(MEMORY as usize);
    map_page(&mut mem, 0, CODE_RA);
    for base in [X, Y] {
        map_ring(&mut mem, base, ring);
    }
    // Eight `ld r9,0(r9)`, then `bdnz` back to the first.
    let mut words = vec![0xE929_0000u32; 8];
    words.push(0x4200_FFE0);
    for (i, w) in words.iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &w.to_be_bytes());
    }
    let (off, ra) = ring;
    for j in 0..2 * LINES {
        let at = ra + LINE * (j % LINES) + if j < LINES { 0 } else { 8 };
        put(&mut mem, at, &element(high, off, j + 1).to_be_bytes());
    }
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
        state.set_gpr(9, element(high, off, 0));
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
            "the chase faulted at pc {:#x}",
            s.pc
        );
        let mut step = rt.step().expect("the step runs");
        rt.commit_step(&mut step).expect("the step commits");
    }
    let s = state(&rt);
    assert_eq!(s.pc, exit, "the chase ran to its exit");
    assert_eq!(s.gpr[9], element(high, off, 0), "the chase closed its ring");
    let halfway = halfway.expect("the chase passed its halfway count");
    (rt.registry().clock(unit).raw() - halfway) as f64 / (8 * ITERATIONS) as f64
}

/// The largest share of the console's mean the model may sit from it.
const TOLERANCE: f64 = 0.20;

#[test]
fn a_load_through_a_second_address_of_a_cached_line_hits_the_cache_as_on_the_console() {
    let (plain, alias) = console();
    assert!(
        (alias - plain).abs() / plain < 0.02,
        "the console: {plain:.2} cycles a load all through X, {alias:.2} through X and Y"
    );
    let cellgov_plain = cycles_a_load(X, PLAIN);
    let cellgov_alias = cycles_a_load(Y, ALIAS);
    assert!(
        (cellgov_plain - plain).abs() / plain <= TOLERANCE,
        "all through X: CellGov {cellgov_plain:.2} cycles a load, the console {plain:.2}"
    );
    assert!(
        (cellgov_alias - alias).abs() / alias <= TOLERANCE,
        "through X and Y: CellGov {cellgov_alias:.2} cycles a load, the console {alias:.2}"
    );
}
