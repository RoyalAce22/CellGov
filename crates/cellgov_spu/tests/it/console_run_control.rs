//! SPU run control held to the `spu_run_control` probe's committed
//! capture: a met halt, a store over instructions ahead of it, branches
//! that set the interrupt-enable bit, and the SPRs, each run once on a
//! raw SPU. Where CellGov models the console's answer, the tests assert
//! agreement. They name the two answers it does not model: the
//! instructions a halt lets run before it stops the SPU, and the fetched
//! lines a store does not reach.

use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{SPU_STATUS_H, SPU_STATUS_P};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/spu_run_control/ps3/cech20-cex-493/observation.json";

const ENTRIES: usize = 21;
const ENTRY_STRIDE: u32 = 0x800;
const SLOTS_AT: u32 = 0x1_0000;
const SLOT_STRIDE: u32 = 0x100;
const PATCH_AT: u32 = 0xF000;
const UNSTORED: u32 = 0xEEEE_EEEE;
/// The stop code of the quadword the prefetch entries store over X.
const PATCH_STOP: u32 = 0x3100;
/// The bytes of one fetched line.
const LINE: u32 = 0x80;

/// One entry's record: (SPU_Status, SPU_NPC, class 0 status, slots).
#[derive(Debug, Clone, Copy)]
struct Record {
    status: u32,
    npc: u32,
    class0: u64,
    slot: [u32; 16],
}

fn console() -> Vec<Record> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    (0..ENTRIES)
        .map(|k| {
            let at = 16 + k * 80;
            Record {
                status: word(at),
                npc: word(at + 4),
                class0: u64::from(word(at + 8)) << 32 | u64::from(word(at + 12)),
                slot: std::array::from_fn(|i| word(at + 16 + i * 4)),
            }
        })
        .collect()
}

/// Words placed at increasing addresses from an entry's base.
struct Code {
    pc: u32,
    words: Vec<(u32, u32)>,
}

impl Code {
    fn at(base: u32) -> Self {
        Code {
            pc: base,
            words: Vec::new(),
        }
    }

    fn put(&mut self, word: u32) -> u32 {
        let at = self.pc;
        self.words.push((at, word));
        self.pc += 4;
        at
    }

    fn ri16(&mut self, opcode: u32, rt: u32, target: u32) -> u32 {
        let rel = ((target as i32 - self.pc as i32) >> 2) as u32 & 0xFFFF;
        self.put(opcode << 23 | rel << 7 | rt)
    }

    fn il(&mut self, rt: u32, imm: u32) {
        self.put(0b0_1000_0001 << 23 | (imm & 0xFFFF) << 7 | rt);
    }

    fn ai(&mut self, rt: u32, ra: u32, imm: u32) {
        self.put(0x1C00_0000 | (imm & 0x3FF) << 14 | ra << 7 | rt);
    }

    fn stqr(&mut self, rt: u32, target: u32) {
        self.ri16(0b0_0100_0111, rt, target);
    }

    fn lqr(&mut self, rt: u32, target: u32) {
        self.ri16(0b0_0110_0111, rt, target);
    }

    fn rdch(&mut self, rt: u32, channel: u32) {
        self.put(0x01A0_0000 | channel << 7 | rt);
    }
}

const NOP: u32 = 0x4020_0000;
const LNOP: u32 = 0x0020_0000;
const SYNC: u32 = 0x0040_0000;
const DSYNC: u32 = 0x0060_0000;

fn slot(k: usize, n: u32) -> u32 {
    SLOTS_AT + k as u32 * SLOT_STRIDE + n * 0x10
}

fn base(k: usize) -> u32 {
    k as u32 * ENTRY_STRIDE
}

/// Entries 0 and 1: `heq $9, $9` with $9 = 0, then a loop that counts
/// $21 to 500 and stores the count to slot 0.
fn halt(k: usize, odd: bool) -> (Code, u32) {
    let mut c = Code::at(base(k));
    c.il(21, 0);
    c.il(9, 0);
    if odd {
        c.put(NOP);
    }
    let heq = c.put(0x7B00_0000 | 9 << 14 | 9 << 7);
    let top = c.pc;
    c.ai(21, 21, 1);
    c.stqr(21, slot(k, 0));
    c.put(0x7C00_0000 | 500 << 14 | 21 << 7 | 22);
    c.ri16(0b0_0100_0000, 22, top);
    c.put(0x2000 + k as u32);
    (c, heq)
}

/// The prefetch entries: a store of the patch quadword over X, `d`
/// quadwords of nops ahead of it, or right after `fence` when one is
/// given. Returns the code and X.
fn prefetch(k: usize, d: u32, fence: Option<u32>) -> (Code, u32) {
    let mut c = Code::at(base(k));
    c.il(20, 0);
    c.lqr(10, PATCH_AT);
    let x = match fence {
        Some(_) => c.pc + 8,
        None => c.pc + 8 + d * 16,
    };
    if fence.is_none() {
        c.put(NOP);
    }
    c.stqr(10, x);
    match fence {
        Some(fence) => {
            c.put(fence);
        }
        None => {
            for _ in 0..d {
                for w in [NOP, LNOP, NOP, LNOP] {
                    c.put(w);
                }
            }
        }
    }
    assert_eq!(c.pc, x);
    for _ in 0..4 {
        c.ai(20, 20, 1);
    }
    c.stqr(20, slot(k, 0));
    c.lqr(11, x);
    c.stqr(11, slot(k, 1));
    c.put(0x2000 + k as u32);
    (c, x)
}

/// The prefetch entries: (entry, d, fence).
const PREFETCH: [(usize, u32, Option<u32>); 17] = [
    (2, 0, None),
    (3, 1, None),
    (4, 2, None),
    (5, 4, None),
    (6, 8, None),
    (7, 16, None),
    (8, 32, None),
    (9, 64, None),
    (10, 0, Some(SYNC)),
    (11, 0, Some(DSYNC)),
    (14, 9, None),
    (15, 10, None),
    (16, 11, None),
    (17, 12, None),
    (18, 13, None),
    (19, 14, None),
    (20, 15, None),
];

/// Entry 12: branches with E, D and E, D, D and E, E, D, each to the next
/// word and followed by a store of SPU_RdMachStat.
fn interrupt_enable() -> Code {
    let k = 12;
    let mut c = Code::at(base(k));
    for (n, bits) in [0x4_0000, 0xC_0000, 0x8_0000, 0xC_0000, 0x4_0000, 0x8_0000]
        .into_iter()
        .enumerate()
    {
        let next = c.pc + 8;
        c.put(0b010_0001 << 25 | next << 7 | 5);
        c.put(0x3500_0000 | bits | 5 << 7);
        c.rdch(3, 13);
        c.stqr(3, slot(k, n as u32));
    }
    c.put(0x2000 + k as u32);
    c
}

/// Entry 13: mfspr of SPRs 0-7, then mtspr spr0 and mfspr spr0.
fn sprs() -> Code {
    let k = 13;
    let mut c = Code::at(base(k));
    c.il(9, 0x1234);
    for n in 0..8 {
        c.il(3, 0x55);
        c.put(0x0180_0000 | n << 7 | 3);
        c.stqr(3, slot(k, n));
    }
    c.put(0x2180_0000 | 9);
    c.il(3, 0x55);
    c.put(0x0180_0003);
    c.stqr(3, slot(k, 8));
    c.put(0x2000 + k as u32);
    c
}

/// Runs `code` from its base on a fresh SPU: (SPU_Status, SPU_NPC, slots).
fn run(k: usize, code: &Code) -> (u32, u32, [u32; 16]) {
    let mut spu = SpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    let ls = &mut spu.state_mut().ls;
    for &(at, w) in &code.words {
        ls[at as usize..at as usize + 4].copy_from_slice(&w.to_be_bytes());
    }
    for i in 0..4 {
        let at = PATCH_AT as usize + i * 4;
        ls[at..at + 4].copy_from_slice(&PATCH_STOP.to_be_bytes());
    }
    let slots = slot(k, 0) as usize;
    for i in 0..64 {
        ls[slots + i * 4..slots + i * 4 + 4].copy_from_slice(&UNSTORED.to_be_bytes());
    }
    spu.state_mut().pc = base(k);
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    let result = spu.run_until_yield(
        Budget::new(10_000),
        &ExecutionContext::new(&mem),
        &mut effects,
    );
    assert_eq!(result.yield_reason, YieldReason::Finished, "entry {k}");
    let stop = spu.stop_registers().expect("a stopped SPU");
    let ls = &spu.state().ls;
    let slot = std::array::from_fn(|i| {
        let at = slots + i * 16;
        u32::from_be_bytes(ls[at..at + 4].try_into().expect("4"))
    });
    (stop.status, stop.npc, slot)
}

#[test]
fn no_entry_latches_a_class_0_status_on_the_console() {
    for (k, record) in console().iter().enumerate() {
        assert_eq!(record.class0, 0, "entry {k}");
    }
}

#[test]
fn a_branch_with_d_and_e_disables_interrupts_as_on_the_console() {
    let record = console()[12];
    assert_eq!(
        record.slot[..6],
        [1, 0, 0, 0, 1, 0],
        "the console's IE reads"
    );
    let (status, npc, slot) = run(12, &interrupt_enable());
    assert_eq!((status, npc), (record.status, record.npc));
    assert_eq!(slot, record.slot);
}

#[test]
fn mfspr_reads_zero_and_mtspr_has_no_effect_as_on_the_console() {
    let record = console()[13];
    assert_eq!(record.slot[..9], [0; 9], "the console's SPR reads");
    let (status, npc, slot) = run(13, &sprs());
    assert_eq!((status, npc), (record.status, record.npc));
    assert_eq!(slot, record.slot);
}

/// The console's fetch holds the line of the store and the line after it:
/// a store to X in either runs the old X, a store past them or behind
/// `sync` runs the new X, and `dsync` leaves the old one. CellGov's
/// fetch sees every store, so it runs the new X where the console ran
/// the old one.
#[test]
fn a_store_over_the_next_fetched_line_is_not_seen_on_the_console_only() {
    let console = console();
    let mut held = 0;
    for (k, d, fence) in PREFETCH {
        let record = console[k];
        let (code, x) = prefetch(k, d, fence);
        let store_line = (code.words[if fence.is_some() { 2 } else { 3 }].0 - base(k)) / LINE;
        let fetched = (x - base(k)) / LINE <= store_line + 1 && fence != Some(SYNC);
        let patched_stop = PATCH_STOP << 16 | SPU_STATUS_P;
        if fetched {
            assert_eq!(
                record.slot[..2],
                [4, PATCH_STOP],
                "entry {k}: the old X ran"
            );
        } else {
            assert_eq!(record.status, patched_stop, "entry {k}: the new X ran");
            assert_eq!(record.npc, x + 4, "entry {k}");
        }
        let (status, npc, _) = run(k, &code);
        assert_eq!(
            (status, npc),
            (patched_stop, x + 4),
            "entry {k}: CellGov runs the new X"
        );
        if !fetched {
            held += 1;
        }
    }
    assert_eq!(held, 5, "16, 32, 64 and 15 quadwords ahead, and sync");
}

/// The console lets a met halt run on before it stops: in both slots the
/// SPU ran three more instructions and stopped with SPU_NPC 16 bytes past
/// the halt. CellGov stops at the halt, 4 bytes past it.
#[test]
fn a_met_halt_runs_on_before_it_stops_on_the_console_only() {
    let console = console();
    for (k, odd) in [(0, false), (1, true)] {
        let record = console[k];
        let (code, heq) = halt(k, odd);
        assert_eq!(record.status, SPU_STATUS_H, "entry {k}: halted");
        assert_eq!(record.npc, heq + 16, "entry {k}");
        assert_eq!(record.slot[0], 1, "entry {k}: one pass of the loop stored");
        let (status, npc, slot) = run(k, &code);
        assert_eq!((status, npc), (SPU_STATUS_H, heq + 4), "entry {k}: CellGov");
        assert_eq!(slot[0], UNSTORED, "entry {k}: CellGov ran nothing past it");
    }
}
