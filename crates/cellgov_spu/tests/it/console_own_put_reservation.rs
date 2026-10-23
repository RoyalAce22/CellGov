//! An SPU's own MFC commands over its reservation line, held to the
//! committed captures of the `spu_own_put_reservation` and
//! `spu_own_put_timing` probes. Each case takes a reservation with
//! getllar, issues one command, and tries a putllc; the lock-line
//! reservation lost event is read between the steps.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    event, MFC_ATOMIC_STAT_G, MFC_ATOMIC_STAT_S, MFC_CMD, MFC_EAH, MFC_EAL, MFC_GET, MFC_GETLLAR,
    MFC_LSA, MFC_PUT, MFC_PUTL, MFC_PUTLLC, MFC_RD_ATOMIC_STAT, MFC_RD_TAG_STAT, MFC_SDCRF,
    MFC_SDCRST, MFC_SDCRZ, MFC_SIZE, MFC_SNDSIG, MFC_TAG_ID, MFC_WR_TAG_MASK, MFC_WR_TAG_UPDATE,
    SPU_RD_EVENT_STAT, SPU_WR_EVENT_ACK, SPU_WR_EVENT_MASK,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const RESERVATION: &str =
    "../../tests/micro/spu_own_put_reservation/ps3/cech20-cex-493/observation.json";
const TIMING: &str = "../../tests/micro/spu_own_put_timing/ps3/cech20-cex-493/observation.json";

/// The reservation probe's cases, eight slots each.
const CASES: usize = 9;
/// The timing probe's slots: four cases of sixteen.
const TIMING_SLOTS: usize = 64;
const SLOTS_AT: u32 = 0x1000;
const UNSTORED: u32 = 0xEEEE_EEEE;
const BLOCKS_EA_AT: usize = 0x1F00;
/// Where the blocks sit in CellGov's guest memory.
const BLOCKS_EA: u32 = 0x1000;

/// (SPU_Status, SPU_NPC, the first `slots` slots) from a capture.
fn console(path: &str, slots: usize) -> (u32, u32, Vec<u32>) {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    assert_eq!((word(24), word(28)), (0, 0), "no class 0 status");
    let slots = (0..slots).map(|i| word(32 + i * 4)).collect();
    (word(16), word(20), slots)
}

/// A probe's SPU program, word by word from address 0.
#[derive(Default)]
struct Program(Vec<u32>);

impl Program {
    fn pc(&self) -> u32 {
        self.0.len() as u32 * 4
    }
    fn put(&mut self, word: u32) {
        self.0.push(word);
    }
    fn il(&mut self, rt: u32, imm: u32) {
        self.put(0x4080_0000 | (imm & 0xFFFF) << 7 | rt);
    }
    fn ila(&mut self, rt: u32, imm: u32) {
        self.put(0x4200_0000 | (imm & 0x3_FFFF) << 7 | rt);
    }
    fn a(&mut self, rt: u32, ra: u32, rb: u32) {
        self.put(0x1800_0000 | rb << 14 | ra << 7 | rt);
    }
    fn ai(&mut self, rt: u32, ra: u32, imm: u32) {
        self.put(0x1C00_0000 | (imm & 0x3FF) << 14 | ra << 7 | rt);
    }
    fn wrch(&mut self, channel: u8, rt: u32) {
        self.put(0x21A0_0000 | u32::from(channel) << 7 | rt);
    }
    fn rdch(&mut self, rt: u32, channel: u8) {
        self.put(0x01A0_0000 | u32::from(channel) << 7 | rt);
    }
    fn rchcnt(&mut self, rt: u32, channel: u8) {
        self.put(0x01E0_0000 | u32::from(channel) << 7 | rt);
    }
    fn lqa(&mut self, rt: u32, addr: u32) {
        self.put(0x3080_0000 | (addr >> 2 & 0xFFFF) << 7 | rt);
    }
    fn stqa(&mut self, rt: u32, addr: u32) {
        self.put(0x2080_0000 | (addr >> 2 & 0xFFFF) << 7 | rt);
    }

    /// Both probes' setup: $11 the blocks' address, the constants, and
    /// SPU_WrEventMask = Lr.
    fn prelude(&mut self) {
        self.lqa(11, BLOCKS_EA_AT as u32);
        for (rt, imm) in [
            (9, 0),
            (10, 0x2000),
            (15, 1),
            (16, 2),
            (17, 2),
            (18, event::LR),
            (40, MFC_GETLLAR),
            (41, MFC_PUTLLC),
        ] {
            self.il(rt, imm);
        }
        self.wrch(MFC_EAH, 9);
        self.wrch(SPU_WR_EVENT_ACK, 18);
        self.wrch(SPU_WR_EVENT_MASK, 18);
    }

    /// Reads the event count and, when it is not 0, the event bits, which
    /// it acks; stores the count to `at` and the bits to `at + 0x10`.
    fn drain(&mut self, at: u32) {
        self.rchcnt(31, SPU_RD_EVENT_STAT);
        self.il(33, 0);
        // brz $31 over the read and the ack.
        self.put(0x2000_0000 | 3 << 7 | 31);
        self.rdch(33, SPU_RD_EVENT_STAT);
        self.wrch(SPU_WR_EVENT_ACK, 33);
        self.stqa(31, at);
        self.stqa(33, at + 0x10);
    }

    /// $12 = the first line of block `k`.
    fn line(&mut self, k: u32) {
        self.ila(19, k * 256);
        self.a(12, 11, 19);
    }

    fn getllar(&mut self, at: u32) {
        self.wrch(MFC_LSA, 10);
        self.wrch(MFC_EAL, 12);
        self.wrch(MFC_CMD, 40);
        self.rdch(20, MFC_RD_ATOMIC_STAT);
        self.stqa(20, at);
    }

    fn putllc(&mut self, at: u32) {
        self.wrch(MFC_LSA, 10);
        self.wrch(MFC_EAL, 12);
        self.wrch(MFC_CMD, 41);
        self.rdch(27, MFC_RD_ATOMIC_STAT);
        self.stqa(27, at);
    }

    /// Issues `op` of `size` bytes between 0x3000 and the address in `ea`,
    /// under tag 1.
    fn issue(&mut self, ea: u32, size: u32, op: u32) {
        self.il(23, 0x3000);
        self.wrch(MFC_LSA, 23);
        self.wrch(MFC_EAL, ea);
        self.il(22, size);
        self.wrch(MFC_SIZE, 22);
        self.wrch(MFC_TAG_ID, 15);
        self.il(24, op);
        self.wrch(MFC_CMD, 24);
    }

    fn wait_tag(&mut self) {
        self.wrch(MFC_WR_TAG_MASK, 16);
        self.wrch(MFC_WR_TAG_UPDATE, 17);
        self.rdch(25, MFC_RD_TAG_STAT);
    }

    fn reserve(&mut self, k: u32) {
        self.line(k);
        self.getllar(slot(k, 0));
        self.drain(slot(k, 1));
    }

    fn command(&mut self, ea: u32, size: u32, op: u32) {
        self.issue(ea, size, op);
        self.wait_tag();
    }

    fn conditional(&mut self, k: u32) {
        self.drain(slot(k, 3));
        self.putllc(slot(k, 5));
        self.drain(slot(k, 6));
    }
}

/// Slot `n` of the reservation probe's case `k`.
fn slot(k: u32, n: u32) -> u32 {
    SLOTS_AT + k * 0x80 + n * 0x10
}

/// Slot `n` of the timing probe's case `k`.
fn timing_slot(k: u32, n: u32) -> u32 {
    SLOTS_AT + k * 0x100 + n * 0x10
}

/// The `spu_own_put_reservation` probe's program.
fn reservation_program() -> Vec<u32> {
    let mut p = Program::default();
    p.prelude();
    p.reserve(0);
    p.conditional(0);
    for (k, next_line, size, op) in [
        (1, false, 16, MFC_PUT),
        (2, true, 16, MFC_PUT),
        (3, false, 128, MFC_SDCRZ),
        (4, false, 128, MFC_SDCRF),
        (5, false, 128, MFC_SDCRST),
    ] {
        p.reserve(k);
        if next_line {
            p.ai(21, 12, 128);
        }
        p.command(if next_line { 21 } else { 12 }, size, op);
        p.conditional(k);
    }
    // Case 6: a list of one 16-byte element at 0x3800 that names the line.
    p.reserve(6);
    p.il(28, 16);
    p.put(0x3EC0_0000 | 4 << 14 | 9 << 7 | 29);
    p.put(0xB000_0000 | 30 << 21 | 28 << 14 | 12 << 7 | 29);
    p.stqa(30, 0x3800);
    p.il(23, 0x3000);
    p.wrch(MFC_LSA, 23);
    p.il(21, 0x3800);
    p.wrch(MFC_EAL, 21);
    p.il(22, 8);
    p.wrch(MFC_SIZE, 22);
    p.wrch(MFC_TAG_ID, 15);
    p.il(24, MFC_PUTL);
    p.wrch(MFC_CMD, 24);
    p.wait_tag();
    p.conditional(6);

    p.reserve(7);
    p.command(12, 16, MFC_GET);
    p.conditional(7);
    p.reserve(8);
    p.conditional(8);
    p.put(0x2000);
    assert!(p.pc() <= SLOTS_AT, "the program ends below the slots");
    p.0
}

/// The `spu_own_put_timing` probe's program.
fn timing_program() -> Vec<u32> {
    let mut p = Program::default();
    p.prelude();
    for (k, op, size, reget) in [
        (0, MFC_SNDSIG, 4, false),
        (1, MFC_PUT, 16, false),
        (2, MFC_PUT, 16, true),
        (3, 0, 0, false),
    ] {
        p.line(k);
        p.getllar(timing_slot(k, 0));
        p.drain(timing_slot(k, 1));
        if op != 0 {
            p.issue(12, size, op);
        }
        p.drain(timing_slot(k, 3));
        if reget {
            p.getllar(timing_slot(k, 5));
        }
        p.wait_tag();
        p.drain(timing_slot(k, 6));
        p.putllc(timing_slot(k, 8));
        p.drain(timing_slot(k, 9));
    }
    p.put(0x2000);
    assert!(p.pc() <= SLOTS_AT, "the program ends below the slots");
    p.0
}

/// Runs `program` over `blocks` 256-byte blocks of 0xA5: (SPU_Status,
/// SPU_NPC, the first `slots` slots).
fn run(program: Vec<u32>, blocks: usize, slots: usize) -> (u32, u32, Vec<u32>) {
    let mut mem = GuestMemory::new(0x2000);
    let range = ByteRange::new(GuestAddr::new(u64::from(BLOCKS_EA)), blocks as u64 * 256)
        .expect("in range");
    mem.apply_commit(range, &vec![0xA5; blocks * 256])
        .expect("the blocks seed");
    let mut rt = Runtime::new(mem, Budget::new(1), 4000);
    let unit = rt.register_unit_with(|id: UnitId| {
        let mut spu = SpuExecutionUnit::new(id);
        let ls = &mut spu.state_mut().ls;
        for (i, w) in program.into_iter().enumerate() {
            ls[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        for i in 0..slots {
            let at = SLOTS_AT as usize + i * 0x10;
            ls[at..at + 4].copy_from_slice(&UNSTORED.to_be_bytes());
        }
        ls[BLOCKS_EA_AT..BLOCKS_EA_AT + 4].copy_from_slice(&BLOCKS_EA.to_be_bytes());
        spu
    });
    let mut finished = false;
    for _ in 0..4000 {
        let Ok(mut step) = rt.step() else { break };
        finished = step.result.yield_reason == YieldReason::Finished;
        rt.commit_step(&mut step).expect("the step commits");
        if finished {
            break;
        }
    }
    assert!(finished, "the SPU stops");
    assert_eq!(rt.take_mfc_exception(), None);
    let spu = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("an SPU unit");
    let stop = spu.stop_registers().expect("a stopped SPU");
    let ls = &spu.state().ls;
    let slots = (0..slots)
        .map(|i| {
            let at = SLOTS_AT as usize + i * 0x10;
            u32::from_be_bytes(ls[at..at + 4].try_into().expect("4"))
        })
        .collect();
    (stop.status, stop.npc, slots)
}

/// [CBEA p:148 s:9.11.1] Lr is set when a snoop external to the MFC resets the reservation, and never for a local action.
/// On the console the SPU's own put, sdcrz and list put over the line
/// reset the reservation and raise Lr; a put to the next line, sdcrf,
/// sdcrst and a get keep it.
#[test]
fn an_spu_own_store_over_its_reservation_line_loses_it_as_on_the_console() {
    let (status, npc, slots) = console(RESERVATION, CASES * 8);
    let lost = [1, 3, 6];
    for k in 0..CASES {
        let case = &slots[k * 8..k * 8 + 8];
        let (event_count, event_bits, putllc) = if lost.contains(&k) {
            (1, event::LR, MFC_ATOMIC_STAT_S)
        } else {
            (0, 0, 0)
        };
        assert_eq!(
            case,
            [
                MFC_ATOMIC_STAT_G,
                0,
                0,
                event_count,
                event_bits,
                putllc,
                0,
                0
            ],
            "case {k} on the console"
        );
    }
    assert_eq!(
        run(reservation_program(), CASES, CASES * 8),
        (status, npc, slots)
    );
}

/// The SPU's own put and sndsig reset its reservation when they land:
/// right after the put issues no Lr is pending, and a getllar taken
/// before the put lands is lost too.
#[test]
fn an_spu_own_put_takes_its_reservation_when_it_lands_as_on_the_console() {
    let (status, npc, slots) = console(TIMING, TIMING_SLOTS);
    let (g, s) = (MFC_ATOMIC_STAT_G, MFC_ATOMIC_STAT_S);
    let lost = [g, 0, 0, 0, 0, UNSTORED, 1, event::LR, s, 0, 0];
    let mut regot = lost;
    regot[5] = g;
    let kept = [g, 0, 0, 0, 0, UNSTORED, 0, 0, 0, 0, 0];
    for (k, expected) in [lost, lost, regot, kept].into_iter().enumerate() {
        assert_eq!(
            slots[k * 16..k * 16 + 11],
            expected,
            "case {k} on the console"
        );
    }
    assert_eq!(run(timing_program(), 4, TIMING_SLOTS), (status, npc, slots));
}
