//! The SPU's channel counts, its mailbox capacities and the MFC commands
//! that need no list, held to the `spu_tables` probe's committed capture
//! from a raw SPU on the reference console.
//!
//! - Every channel's `rchcnt` at idle, 0 to 127.
//! - Five problem-state stores into the four-entry inbound mailbox, the
//!   `SPU_Mbox_Stat` after each, and what the SPU then reads.
//! - The outbound mailbox's count before and after one write, and the
//!   problem-state view of it.
//! - Sixteen MFC commands on 128-byte lines: the put and get forms with
//!   their barrier, fence and replace-hint variants, the three ordering
//!   commands, and the four atomic commands. Each is checked by the tag
//!   or atomic status it leaves and by what it moved.

use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    self, MFC_BARRIER, MFC_CMD, MFC_EAH, MFC_EAL, MFC_EIEIO, MFC_GET, MFC_GETB, MFC_GETF,
    MFC_GETLLAR, MFC_LSA, MFC_PUT, MFC_PUTB, MFC_PUTF, MFC_PUTLLC, MFC_PUTLLUC, MFC_PUTQLLUC,
    MFC_PUTR, MFC_PUTRB, MFC_PUTRF, MFC_RD_ATOMIC_STAT, MFC_RD_TAG_STAT, MFC_SIZE, MFC_SYNC,
    MFC_TAG_ID, MFC_TAG_UPDATE_ALL, MFC_WR_TAG_MASK, MFC_WR_TAG_UPDATE, SPU_IN_MBOX_OFFSET,
    SPU_MBOX_STAT_OFFSET, SPU_NPC_OFFSET, SPU_OUT_MBOX_OFFSET, SPU_RD_IN_MBOX, SPU_RUN_CNTL_OFFSET,
    SPU_STATUS_OFFSET, SPU_STATUS_P, SPU_WR_OUT_MBOX,
};
use cellgov_spu::exec::{execute, SpuFault, SpuStepOutcome};
use cellgov_spu::instruction::SpuInstruction;
use cellgov_spu::state::SpuState;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

use crate::raw_spu_window::{
    create_ops, poll_stopped, read, register, run_ops, runtime, Op, WINDOW,
};

const CONSOLE: &str = "../../tests/micro/spu_tables/ps3/cech20-cex-493/observation.json";

/// Where each part of `spu/main.S` starts.
const MBOX_IN_AT: u32 = 0x800;
const MBOX_OUT_AT: u32 = 0x900;
const MFC_AT: u32 = 0xA00;
/// Where the SPU stores what it reads.
const MBOX_IN_LS: u32 = 0x2800;
const MBOX_OUT_LS: u32 = 0x2880;
const TAG_STAT_LS: u32 = 0x3000;
const ATOMIC_STAT_LS: u32 = 0x3100;
const LINES_LS: u32 = 0x4000;
const BUFFER_EA_AT: u32 = 0x1F00;
const LINES: usize = 16;
const LINE: u32 = 0x80;
const UNSTORED: u32 = 0xEEEE_EEEE;
/// What the PPU writes to the inbound mailbox, plus 1..=5.
const IN_MESSAGE: u32 = 0x1571_0000;
/// What the SPU writes to the outbound mailbox.
const OUT_MESSAGE: u32 = 0x0BB0_1571;
/// Where the buffer sits in CellGov's guest memory.
const BUFFER_EA: u32 = 0x1000;

/// The probe's payload, as big-endian words.
fn words() -> Vec<u32> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let w: Vec<u32> = p
        .chunks_exact(4)
        .map(|b| u32::from_be_bytes(b.try_into().expect("4 bytes")))
        .collect();
    assert_eq!((w[0], w[1]), (0, 0), "the probe ran every step");
    w
}

/// `stop` with signal `code`, as SPU_Status reports it.
fn stopped(code: u32) -> u32 {
    code << 16 | SPU_STATUS_P
}

#[derive(Default)]
struct Program(Vec<u32>);

impl Program {
    fn put(&mut self, word: u32) {
        self.0.push(word);
    }
    fn il(&mut self, rt: u32, imm: u32) {
        self.put(0x4080_0000 | (imm & 0xFFFF) << 7 | rt);
    }
    fn ilhu(&mut self, rt: u32, imm: u32) {
        self.put(0x4100_0000 | (imm & 0xFFFF) << 7 | rt);
    }
    fn iohl(&mut self, rt: u32, imm: u32) {
        self.put(0x6080_0000 | (imm & 0xFFFF) << 7 | rt);
    }
    /// `a rt, ra, rb`.
    fn a(&mut self, rt: u32, ra: u32, rb: u32) {
        self.put(0x1800_0000 | rb << 14 | ra << 7 | rt);
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
    fn stop(&mut self, code: u32) {
        self.put(code);
    }
    /// Points the parameter channels at line `k`: 16 bytes, tag 1.
    fn params(&mut self, k: u32) {
        self.il(4, LINES_LS + k * LINE);
        self.wrch(MFC_LSA, 4);
        self.il(6, k * LINE);
        self.a(5, 11, 6);
        self.wrch(MFC_EAL, 5);
        self.il(7, 16);
        self.wrch(MFC_SIZE, 7);
        self.il(8, 1);
        self.wrch(MFC_TAG_ID, 8);
    }
    /// Tagged command `k`, then a wait for every tag group whose status
    /// goes to slot `k`.
    fn tagged(&mut self, k: u32, op: u32) {
        self.params(k);
        self.il(10, op);
        self.wrch(MFC_CMD, 10);
        self.wrch(MFC_WR_TAG_MASK, 15);
        self.wrch(MFC_WR_TAG_UPDATE, 16);
        self.rdch(25, MFC_RD_TAG_STAT);
        self.stqa(25, TAG_STAT_LS + k * 16);
    }
    /// Atomic command `k` on line `ln`; its status goes to slot `k`.
    fn atomic(&mut self, k: u32, op: u32, ln: u32) {
        self.params(ln);
        self.il(10, op);
        self.wrch(MFC_CMD, 10);
        self.rdch(12, MFC_RD_ATOMIC_STAT);
        self.stqa(12, ATOMIC_STAT_LS + k * 16);
    }
}

/// The in-mailbox part of `spu/main.S`, from 0x800.
fn mbox_in() -> Vec<u32> {
    let mut p = Program::default();
    p.rchcnt(3, SPU_RD_IN_MBOX);
    p.stqa(3, MBOX_IN_LS);
    for i in 0..4 {
        p.rdch(4, SPU_RD_IN_MBOX);
        p.stqa(4, MBOX_IN_LS + 0x10 + i * 0x10);
    }
    p.rchcnt(3, SPU_RD_IN_MBOX);
    p.stqa(3, MBOX_IN_LS + 0x50);
    p.stop(0x2002);
    p.0
}

/// The out-mailbox part of `spu/main.S`, from 0x900.
fn mbox_out() -> Vec<u32> {
    let mut p = Program::default();
    p.rchcnt(3, SPU_WR_OUT_MBOX);
    p.stqa(3, MBOX_OUT_LS);
    p.ilhu(4, OUT_MESSAGE >> 16);
    p.iohl(4, OUT_MESSAGE & 0xFFFF);
    p.wrch(SPU_WR_OUT_MBOX, 4);
    p.rchcnt(3, SPU_WR_OUT_MBOX);
    p.stqa(3, MBOX_OUT_LS + 0x10);
    p.stop(0x2003);
    p.0
}

/// The MFC part of `spu/main.S`, from 0xA00.
fn mfc() -> Vec<u32> {
    let mut p = Program::default();
    p.lqa(11, BUFFER_EA_AT);
    p.il(15, u32::MAX);
    p.il(16, MFC_TAG_UPDATE_ALL);
    p.il(9, 0);
    p.wrch(MFC_EAH, 9);
    for k in 0..LINES as u32 {
        p.ilhu(3, 0x5000);
        p.iohl(3, k);
        p.stqa(3, LINES_LS + k * LINE);
    }
    let tagged = [
        MFC_PUT,
        MFC_PUTB,
        MFC_PUTF,
        MFC_PUTR,
        MFC_PUTRB,
        MFC_PUTRF,
        MFC_GET,
        MFC_GETB,
        MFC_GETF,
        MFC_BARRIER,
        MFC_EIEIO,
        MFC_SYNC,
    ];
    for (k, op) in tagged.into_iter().enumerate() {
        p.tagged(k as u32, op);
    }
    p.atomic(12, MFC_GETLLAR, 12);
    p.ilhu(3, 0x5000);
    p.iohl(3, 13);
    p.stqa(3, LINES_LS + 12 * LINE);
    p.atomic(13, MFC_PUTLLC, 12);
    p.atomic(14, MFC_PUTLLUC, 14);
    p.tagged(15, MFC_PUTQLLUC);
    p.stop(0x2004);
    p.0
}

/// What a fresh SPU's `rchcnt` reads on `channel`, or the refusal.
fn cellgov_count(channel: u8) -> Result<u32, SpuFault> {
    let mut s = SpuState::new();
    match execute(
        &SpuInstruction::Rchcnt { rt: 3, channel },
        &mut s,
        UnitId::new(0),
    ) {
        SpuStepOutcome::Continue => Ok(s.reg_word(3)),
        SpuStepOutcome::Fault(fault) => Err(fault),
        other => panic!("channel {channel}: {other:?}"),
    }
}

/// Every channel counts what it counted on the console.
#[test]
fn every_channel_count_at_idle_is_the_console_s() {
    let w = words();
    assert_eq!(w[4], stopped(0x2001), "the sweep ran to its stop");
    assert_eq!(w[5], 0, "no channel stopped the SPU");
    let console = &w[38..166];
    assert_eq!(console[usize::from(spu::MFC_CMD)], spu::MFC_SPU_QUEUE_DEPTH);
    for channel in spu::SPU_RESERVED_CHANNELS_COUNTING_ONE {
        assert!(spu::is_reserved_channel(channel));
        assert_eq!(console[usize::from(channel)], 1, "channel {channel}");
    }
    for channel in 0..128u8 {
        let want = console[usize::from(channel)];
        assert_eq!(cellgov_count(channel), Ok(want), "channel {channel}");
    }
}

/// Writes `program` into the raw SPU's local store at `at`, then runs it
/// from there to its stop.
fn load_and_run(program: &[u32], at: u32) -> Vec<Op> {
    let mut ops: Vec<Op> = program
        .iter()
        .enumerate()
        .map(|(i, &w)| Op::Store(WINDOW + u64::from(at) + 4 * i as u64, w))
        .collect();
    ops.extend([
        Op::Store(register(SPU_NPC_OFFSET), at),
        Op::Store(register(SPU_RUN_CNTL_OFFSET), 1),
        poll_stopped(),
    ]);
    ops
}

fn local(rt: &Runtime, at: u32) -> u32 {
    read(rt, WINDOW + u64::from(at))
}

/// The inbound mailbox holds four entries and a fifth store replaces the
/// last; the outbound mailbox holds one. `SPU_Mbox_Stat` and the SPU's
/// counts follow each step as they did on the console.
#[test]
fn the_mailboxes_fill_and_drain_as_on_the_console() {
    let w = words();
    assert_eq!(w[171], stopped(0x2002));
    assert_eq!(w[180], stopped(0x2003));
    assert_eq!(w[184], OUT_MESSAGE);

    let stores: Vec<Op> = (1..=5)
        .map(|i| Op::Store(register(SPU_IN_MBOX_OFFSET), IN_MESSAGE + i))
        .collect();
    let mut ops = create_ops();
    ops.extend(stores.iter().copied());
    ops.extend(load_and_run(&mbox_in(), MBOX_IN_AT));
    ops.extend(load_and_run(&mbox_out(), MBOX_OUT_AT));
    ops.push(Op::Load(register(SPU_OUT_MBOX_OFFSET)));
    let (mut rt, ppu) = runtime(ops);

    run_ops(&mut rt, ppu, create_ops().len());
    for (i, &want) in w[166..171].iter().enumerate() {
        run_ops(&mut rt, ppu, 1);
        assert_eq!(
            read(&rt, register(SPU_MBOX_STAT_OFFSET)),
            want,
            "store {}",
            i + 1
        );
    }
    run_ops(&mut rt, ppu, mbox_in().len() + 3);
    assert_eq!(read(&rt, register(SPU_STATUS_OFFSET)), w[171]);
    assert_eq!(local(&rt, MBOX_IN_LS), w[172], "the count before the reads");
    for i in 0..4 {
        assert_eq!(
            local(&rt, MBOX_IN_LS + 0x10 + i * 0x10),
            w[173 + i as usize],
            "read {i}"
        );
    }
    assert_eq!(
        local(&rt, MBOX_IN_LS + 0x50),
        w[177],
        "the count after them"
    );
    assert_eq!(read(&rt, register(SPU_MBOX_STAT_OFFSET)), w[178]);
    assert_eq!(w[179], w[178], "nothing changed before the out-mailbox run");

    run_ops(&mut rt, ppu, mbox_out().len() + 3);
    assert_eq!(read(&rt, register(SPU_STATUS_OFFSET)), w[180]);
    assert_eq!(
        local(&rt, MBOX_OUT_LS),
        w[181],
        "the count before the write"
    );
    assert_eq!(local(&rt, MBOX_OUT_LS + 0x10), w[182], "the count after it");
    assert_eq!(read(&rt, register(SPU_MBOX_STAT_OFFSET)), w[183]);
    assert_eq!(read(&rt, register(SPU_OUT_MBOX_OFFSET)), w[184]);
    run_ops(&mut rt, ppu, 1);
    assert_eq!(read(&rt, register(SPU_MBOX_STAT_OFFSET)), w[185]);
}

/// One MFC run's answers: SPU_Status, SPU_NPC, and per line the tag
/// status, the atomic status, local-store word 0 and buffer word 0.
type MfcRun = (u32, u32, [[u32; LINES]; 4]);

fn console_mfc(w: &[u32]) -> MfcRun {
    assert_eq!((w[188], w[189]), (0, 0), "no class 0 status");
    let line = |base: usize| -> [u32; LINES] { std::array::from_fn(|k| w[base + k]) };
    (w[186], w[187], [line(192), line(208), line(224), line(240)])
}

fn run_mfc() -> MfcRun {
    let mut rt = Runtime::new(GuestMemory::new(0x2000), Budget::new(1), 20_000);
    for k in 0..LINES as u32 {
        let at =
            ByteRange::new(GuestAddr::new(u64::from(BUFFER_EA + k * LINE)), 4).expect("in range");
        rt.place_bytes(AddressSpaceId::BOOT, at, &(0xA000_0000 | k).to_be_bytes())
            .expect("the buffer takes the word");
    }
    let unit = rt.register_unit_with(|id: UnitId| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        let ls = &mut state.ls;
        let code = MFC_AT as usize;
        for (i, w) in mfc().into_iter().enumerate() {
            ls[code + i * 4..code + i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        for at in (TAG_STAT_LS..TAG_STAT_LS + 0x200).step_by(4) {
            let at = at as usize;
            ls[at..at + 4].copy_from_slice(&UNSTORED.to_be_bytes());
        }
        let ea = BUFFER_EA_AT as usize;
        ls[ea..ea + 4].copy_from_slice(&BUFFER_EA.to_be_bytes());
        state.pc = MFC_AT;
        spu
    });
    let mut finished = false;
    for _ in 0..20_000 {
        let Ok(step) = rt.step() else { break };
        finished = step.result.yield_reason == YieldReason::Finished;
        rt.commit_unchecked(&step.result, &step.effects)
            .expect("the step commits");
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
    let ls_word = |at: u32| {
        let at = at as usize;
        u32::from_be_bytes(ls[at..at + 4].try_into().expect("4"))
    };
    let buffer = |k: u32| {
        let range =
            ByteRange::new(GuestAddr::new(u64::from(BUFFER_EA + k * LINE)), 4).expect("in range");
        let bytes = rt.memory().read(range).expect("readable");
        u32::from_be_bytes(bytes.try_into().expect("4"))
    };
    let per_line =
        |f: &dyn Fn(u32) -> u32| -> [u32; LINES] { std::array::from_fn(|k| f(k as u32)) };
    (
        stop.status,
        stop.npc,
        [
            per_line(&|k| ls_word(TAG_STAT_LS + k * 16)),
            per_line(&|k| ls_word(ATOMIC_STAT_LS + k * 16)),
            per_line(&|k| ls_word(LINES_LS + k * LINE)),
            per_line(&buffer),
        ],
    )
}

/// Each put moved its local line out and each get moved the buffer's line
/// in, every tag wait saw all groups complete, the ordering commands moved
/// nothing, and the atomic commands left the statuses and the lines the
/// console left.
#[test]
fn the_mfc_commands_move_and_report_as_on_the_console() {
    let w = words();
    let console = console_mfc(&w);
    let (status, npc, [_, atomic, _, buffer]) = console;
    assert_eq!(status, stopped(0x2004));
    assert_eq!(
        npc,
        MFC_AT + mfc().len() as u32 * 4,
        "past the closing stop"
    );
    assert_eq!(
        atomic[12..15],
        [spu::MFC_ATOMIC_STAT_G, 0, spu::MFC_ATOMIC_STAT_U],
        "getllar, a putllc that held its reservation, putlluc"
    );
    assert_eq!(buffer[12], 0x5000_000D, "the putllc stored");
    assert_eq!(run_mfc(), console);
}
