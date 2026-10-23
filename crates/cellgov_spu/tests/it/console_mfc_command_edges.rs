//! MFC command edges held to the `spu_mfc_command_edges` probe's
//! committed capture: a second put that writes only MFC_Cmd, a list-stall
//! ack with no list stalled, and the reserved tag-update condition.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_EAH, MFC_EAL, MFC_GET, MFC_LSA, MFC_PUT, MFC_RD_LIST_STALL_STAT, MFC_RD_TAG_STAT,
    MFC_SIZE, MFC_TAG_ID, MFC_TAG_UPDATE_ALL, MFC_TAG_UPDATE_RESERVED, MFC_WR_LIST_STALL_ACK,
    MFC_WR_TAG_MASK, MFC_WR_TAG_UPDATE,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/spu_mfc_command_edges/ps3/cech20-cex-493/observation.json";

const SLOTS: usize = 8;
const SLOTS_AT: u32 = 0x2000;
const UNSTORED: u32 = 0xEEEE_EEEE;
const BUFFER_EA_AT: usize = 0x1F00;
/// Where the buffer sits in CellGov's guest memory.
const BUFFER_EA: u32 = 0x1000;

/// One run's answers: (SPU_Status, SPU_NPC, slots, buffer words at +0
/// and +0x10).
type Answers = (u32, u32, Vec<u32>, [u32; 2]);

fn console() -> Answers {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    assert_eq!((word(24), word(28)), (0, 0), "no class 0 status");
    (
        word(16),
        word(20),
        (0..SLOTS).map(|i| word(40 + i * 4)).collect(),
        [word(72), word(76)],
    )
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
    /// Waits for every tag group and stores the tag status to slot `n`.
    fn wait_all(&mut self, n: u32) {
        self.wrch(MFC_WR_TAG_MASK, 16);
        self.wrch(MFC_WR_TAG_UPDATE, 17);
        self.rdch(25, MFC_RD_TAG_STAT);
        self.stqa(25, slot(n));
    }
}

fn slot(n: u32) -> u32 {
    SLOTS_AT + n * 0x10
}

/// The probe's SPU program, word by word from address 0.
fn program() -> Vec<u32> {
    let mut p = Program::default();
    p.lqa(11, BUFFER_EA_AT as u32);
    p.il(9, 0);
    p.wrch(MFC_EAH, 9);
    p.il(15, 1);
    p.il(16, u32::MAX);
    p.il(17, MFC_TAG_UPDATE_ALL);

    // A put with every parameter, then one that writes only MFC_Cmd.
    p.il(3, 0x1111);
    p.stqa(3, 0x3000);
    p.il(4, 0x3000);
    p.wrch(MFC_LSA, 4);
    p.wrch(MFC_EAL, 11);
    p.il(5, 16);
    p.wrch(MFC_SIZE, 5);
    p.wrch(MFC_TAG_ID, 15);
    p.il(6, MFC_PUT);
    p.wrch(MFC_CMD, 6);
    p.wait_all(0);
    p.il(3, 0x2222);
    p.stqa(3, 0x3000);
    p.wrch(MFC_CMD, 6);
    p.wait_all(1);

    // A stall ack with no list stalled, then a get on that tag.
    p.il(7, 5);
    p.wrch(MFC_WR_LIST_STALL_ACK, 7);
    p.rchcnt(8, MFC_RD_LIST_STALL_STAT);
    p.stqa(8, slot(2));
    p.il(4, 0x3100);
    p.wrch(MFC_LSA, 4);
    p.wrch(MFC_EAL, 11);
    p.wrch(MFC_SIZE, 5);
    p.wrch(MFC_TAG_ID, 7);
    p.il(6, MFC_GET);
    p.wrch(MFC_CMD, 6);
    p.wait_all(3);

    // The reserved update condition, then a wait for all tags.
    p.il(10, MFC_TAG_UPDATE_RESERVED);
    p.wrch(MFC_WR_TAG_UPDATE, 10);
    p.rchcnt(12, MFC_RD_TAG_STAT);
    p.stqa(12, slot(4));
    // brz $12 over the read and its store.
    p.put(0x2000_0000 | 3 << 7 | 12);
    p.rdch(13, MFC_RD_TAG_STAT);
    p.stqa(13, slot(5));
    p.wait_all(6);
    p.put(0x2000);
    p.0
}

fn run() -> Answers {
    let mut rt = Runtime::new(GuestMemory::new(0x2000), Budget::new(1), 2000);
    let unit = rt.register_unit_with(|id: UnitId| {
        let mut spu = SpuExecutionUnit::new(id);
        let ls = &mut spu.state_mut().ls;
        for (i, w) in program().into_iter().enumerate() {
            ls[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        for n in 0..SLOTS as u32 {
            let at = slot(n) as usize;
            ls[at..at + 4].copy_from_slice(&UNSTORED.to_be_bytes());
        }
        ls[BUFFER_EA_AT..BUFFER_EA_AT + 4].copy_from_slice(&BUFFER_EA.to_be_bytes());
        spu
    });
    let mut finished = false;
    for _ in 0..2000 {
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
    let slots = (0..SLOTS as u32)
        .map(|n| {
            let at = slot(n) as usize;
            u32::from_be_bytes(ls[at..at + 4].try_into().expect("4"))
        })
        .collect();
    let buffer = |offset: u64| {
        let range =
            ByteRange::new(GuestAddr::new(u64::from(BUFFER_EA) + offset), 4).expect("in range");
        let bytes = rt.memory().read(range).expect("readable");
        u32::from_be_bytes(bytes.try_into().expect("4"))
    };
    (stop.status, stop.npc, slots, [buffer(0), buffer(0x10)])
}

/// A second put that writes only MFC_Cmd reuses the last LSA, EAL and
/// size; a stall ack with no list stalled changes nothing; and the
/// reserved tag-update condition requests no update, so the
/// MFC_RdTagStat count stays 0 and a later wait still completes.
#[test]
fn mfc_command_edges_answer_as_on_the_console() {
    let console = console();
    let (_, _, slots, buffer) = &console;
    assert_eq!(buffer[0], 0x2222, "the second put reused the parameters");
    assert_eq!(slots[2], 0, "no stalled list after the ack");
    assert_eq!(slots[4], 0, "the reserved condition requests no update");
    assert_eq!(slots[5], UNSTORED);
    assert_eq!(run(), console);
}
