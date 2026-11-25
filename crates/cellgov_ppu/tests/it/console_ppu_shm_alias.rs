//! Two effective addresses of one real page, held to the `ppu_shm_alias`
//! console capture.
//!
//! The console probe maps one shared-memory segment at X and at Y. The
//! replay is a thread with supervisor registers whose page table maps two
//! pages of one segment, X and Y, to one real page. Each trial writes a
//! fill doubleword through X, then runs the probe's case:
//!
//! - case 0: `lwz` Y, `stw` X, `lwz` Y;
//! - case 1: `lwarx` X, `stwcx.` Y;
//! - case 2: `lwarx` Y, `stwcx.` X;
//! - case 3: `ldarx` X, `stdcx.` Y;
//! - case 4: `lwarx` X, `stwcx.` X.
//!
//! Case 0 counts the trials whose second load reads the stored word, and
//! those whose first load reads the fill. Cases 1 to 4 count the trials
//! that set CR0\[EQ\], and those whose conditional store target then
//! holds the stored value. Case 2 reads that target through Y, and the
//! other cases through X.

use cellgov_event::UnitId;
use cellgov_lv2::hypervisor::HypercallRefusal;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::mmu::Partition;
use cellgov_ppu::supervisor::{SlbEntry, SupervisorState};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_SF, SLB_VALID};
use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome};
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/ppu_shm_alias/ps3/cech20-cex-493/observation.json";

const CASES: usize = 5;

/// The probe's payload: 20 words.
const WORDS: usize = 20;

/// The page table: 2^11 groups at 256 KB, ending at the memory's end.
const HTAB: u64 = 0x4_0000;
const MEMORY: u64 = 0x8_0000;
/// The segment SLB entry 0 maps.
const SEGMENT: u64 = 0x1000_0000;
const VSID: u64 = 0x123;
/// Page 0 of the segment holds the program, at real 0x3000.
const CODE_RA: u64 = 0x3000;
/// Pages 1 and 2, X and Y, both map real 0x5000.
const SHARED_RA: u64 = 0x5000;
/// Page 3 holds each trial's record, at real 0x6000.
const RECORDS_RA: u64 = 0x6000;
/// One record: the CR or first loaded word, then the doubleword read.
const RECORD: u64 = 16;

/// The fill each trial writes through X before its case.
const FILL: u64 = 0x5A5A_5A5A_5A5A_5A5A;

/// The value case `c`'s trial `i` stores.
fn stored(c: usize, i: u64) -> u64 {
    ((c as u64 + 1) << 8) + i
}

/// The offset of case `c`'s doubleword in the shared page.
fn line(c: usize) -> u16 {
    0x100 * c as u16
}

/// The capture's trial count and each case's two counts.
fn console() -> (u64, [[u64; 2]; CASES]) {
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
    assert_ne!(word(4), word(5), "the segment is at two addresses");
    for w in 16..20 {
        assert_eq!(word(w), 0, "teardown word {w} returned 0");
    }
    (
        word(2),
        std::array::from_fn(|c| [word(6 + 2 * c), word(7 + 2 * c)]),
    )
}

fn d_form(op: u32, rt: u32, ra: u32, d: u16) -> u32 {
    op << 26 | rt << 21 | ra << 16 | u32::from(d)
}

fn x_form(rt: u32, rb: u32, xo: u32, rc: u32) -> u32 {
    31 << 26 | rt << 21 | rb << 11 | xo << 1 | rc
}

/// Loads `SEGMENT + low` into `rt`.
fn segment_address(p: &mut Vec<u32>, rt: u32, low: u16) {
    p.push(d_form(15, rt, 0, (SEGMENT >> 16) as u16)); // lis
    p.push(d_form(24, rt, rt, low)); // ori
}

/// Assembles every case's trial loop, each trial's record stored through
/// page 3, and an `sc 1` at the end.
fn program(trials: u64) -> Vec<u32> {
    let mut p = Vec::new();
    // r20 the fill: lis, ori, rldimi r20,r20,32,0.
    p.push(d_form(15, 20, 0, (FILL >> 48) as u16));
    p.push(d_form(24, 20, 20, FILL as u16));
    p.push(30 << 26 | 20 << 21 | 20 << 16 | 3 << 2 | 1 << 1);
    segment_address(&mut p, 28, 0x3000);
    for c in 0..CASES {
        segment_address(&mut p, 30, 0x1000 + line(c));
        segment_address(&mut p, 29, 0x2000 + line(c));
        p.push(d_form(14, 21, 0, stored(c, 0) as u16)); // li r21
        p.push(d_form(14, 27, 0, trials as u16)); // li r27
        p.push(0x7C09_03A6 | 27 << 21); // mtctr r27
        let head = p.len();
        p.push(d_form(62, 20, 30, 0)); // std r20,0(r30)
        p.push(0x7C00_04AC); // sync
        let lwarx = |rb| x_form(9, rb, 20, 0);
        let stwcx = |rb| x_form(21, rb, 150, 1);
        let mfcr = 0x7C00_0026 | 7 << 21;
        match c {
            0 => {
                p.push(d_form(32, 7, 29, 0)); // lwz r7,0(r29)
                p.push(d_form(36, 21, 30, 0)); // stw r21,0(r30)
                p.push(d_form(32, 8, 29, 0)); // lwz r8,0(r29)
            }
            1 => p.extend([lwarx(30), stwcx(29), mfcr, d_form(32, 8, 30, 0)]),
            2 => p.extend([lwarx(29), stwcx(30), mfcr, d_form(32, 8, 29, 0)]),
            3 => p.extend([
                x_form(9, 30, 84, 0),   // ldarx r9,0,r30
                x_form(21, 29, 214, 1), // stdcx. r21,0,r29
                mfcr,
                d_form(58, 8, 30, 0), // ld r8,0(r30)
            ]),
            _ => p.extend([lwarx(30), stwcx(30), mfcr, d_form(32, 8, 30, 0)]),
        }
        p.push(d_form(36, 7, 28, 0)); // stw r7,0(r28)
        p.push(d_form(62, 8, 28, 8)); // std r8,8(r28)
        p.push(d_form(14, 28, 28, RECORD as u16)); // addi r28,r28,16
        p.push(d_form(14, 21, 21, 1)); // addi r21,r21,1
        let back = (head as i32 - p.len() as i32) * 4;
        p.push(16 << 26 | 16 << 21 | (back as u32 & 0xFFFC)); // bdnz head
    }
    // The run continues past a trap, which takes the program interrupt
    // at the kernel's vector. `sc 1` with r11 = 5, a number neither copy
    // of lv1call.h lists, is a hypercall that CellGov's hypervisor does
    // not model, so the run stops there.
    p.push(d_form(14, 11, 0, 5)); // li r11, 5
    p.push(0x4400_0022); // sc 1
    p
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

/// A thread at the segment's first word that translates through SLB entry
/// 0 and the page table.
fn translating_unit(id: UnitId) -> PpuExecutionUnit {
    let mut sup = SupervisorState::new(MSR_SF | MSR_IR | MSR_DR);
    sup.slb[0] = SlbEntry {
        esid_v: SEGMENT | SLB_VALID,
        vsid_flags: VSID << 12,
    };
    sup.partition = Partition::new(HTAB, 1 << 62, 0, Partition::INITIAL.lb()).expect("partition");
    let mut unit = PpuExecutionUnit::new(id);
    unit.state_mut().set_supervisor(Some(sup));
    unit.state_mut().pc = SEGMENT;
    unit
}

/// Runs the program and returns each case's two counts.
fn run(trials: u64) -> [[u64; 2]; CASES] {
    let words = program(trials);
    let words_len = words.len();
    assert!(
        4 * (words.len() as u64) <= 0x1000,
        "the program fits page 0"
    );
    let fixture = ScenarioFixture::builder()
        .memory_size(MEMORY as usize)
        .budget(Budget::new(64))
        .max_steps(100_000)
        .seed_memory(move |mem| {
            for (index, ra) in [
                (0, CODE_RA),
                (1, SHARED_RA),
                (2, SHARED_RA),
                (3, RECORDS_RA),
            ] {
                map_page(mem, index, ra);
            }
            for (i, w) in words.iter().enumerate() {
                put(mem, CODE_RA + 4 * i as u64, &w.to_be_bytes());
            }
        })
        .register(|rt| {
            rt.register_unit_with(translating_unit);
        })
        .build();
    let result = runner::run(fixture);
    let last = SEGMENT + 4 * (words_len as u64 - 1);
    assert_eq!(
        result.outcome,
        ScenarioOutcome::HypercallRefused(HypercallRefusal::Unmodelled {
            number: 5,
            unit: UnitId::new(0),
            pc: last,
        }),
        "the program ends at its hypervisor call: {:?}",
        result.first_fault
    );
    assert_eq!(result.first_fault, None, "no fault before the call");
    let m = &result.final_memory;
    let mut counts = [[0u64; 2]; CASES];
    for (c, count) in counts.iter_mut().enumerate() {
        for i in 0..trials {
            let at = (RECORDS_RA + RECORD * (c as u64 * trials + i)) as usize;
            let first = u32::from_be_bytes(m[at..at + 4].try_into().expect("4"));
            let read = u64::from_be_bytes(m[at + 8..at + 16].try_into().expect("8"));
            let v = stored(c, i);
            let [a, b] = if c == 0 {
                [read == v, u64::from(first) == FILL >> 32]
            } else {
                [first >> 29 & 1 == 1, read == v]
            };
            count[0] += u64::from(a);
            count[1] += u64::from(b);
        }
    }
    counts
}

#[test]
fn two_effective_addresses_of_one_real_page_share_its_bytes_and_its_reservation_as_on_the_console()
{
    let (trials, console) = console();
    assert_eq!(
        console, [[trials; 2]; CASES],
        "the console's answer in every trial"
    );
    assert_eq!(run(trials), console);
}
