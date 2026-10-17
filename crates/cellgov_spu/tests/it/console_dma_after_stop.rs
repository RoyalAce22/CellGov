//! Transfers queued before a stop, held to the `spu_dma_after_stop`
//! probe's committed capture: four 16 KB puts under one tag, then a stop
//! that does not wait for the tag.

use cellgov_core::{Runtime, StepError};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{MFC_CMD, MFC_EAH, MFC_EAL, MFC_LSA, MFC_PUT, MFC_SIZE, MFC_TAG_ID};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/spu_dma_after_stop/ps3/cech20-cex-493/observation.json";

const CHUNK: u32 = 0x4000;
const CHUNKS: u32 = 4;
const DATA_AT: u32 = 0x4000;
const DATA_WORD: u32 = 0x5A5A_5A5A;
const BUFFER_EA_AT: usize = 0x1F00;
/// Where the buffer sits in CellGov's guest memory.
const BUFFER_EA: u32 = 0x1_0000;
const TAG: u32 = 1;

/// One run's answers: (SPU_Status, SPU_NPC, tag status once the MFC has
/// settled, the last and the first word of each 16 KB of the buffer).
type Answers = (u32, u32, u32, Vec<u32>, Vec<u32>);

fn console() -> Answers {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    let words = |at: usize| (0..CHUNKS as usize).map(|i| word(at + i * 4)).collect();
    (word(16), word(20), word(52), words(56), words(72))
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
    fn ila(&mut self, rt: u32, imm: u32) {
        self.put(0x4200_0000 | (imm & 0x3_FFFF) << 7 | rt);
    }
    fn a(&mut self, rt: u32, ra: u32, rb: u32) {
        self.put(0x1800_0000 | rb << 14 | ra << 7 | rt);
    }
    fn wrch(&mut self, channel: u8, rt: u32) {
        self.put(0x21A0_0000 | u32::from(channel) << 7 | rt);
    }
    fn lqa(&mut self, rt: u32, addr: u32) {
        self.put(0x3080_0000 | (addr >> 2 & 0xFFFF) << 7 | rt);
    }
}

/// The probe's SPU program, word by word from address 0.
fn program() -> Vec<u32> {
    let mut p = Program::default();
    p.lqa(11, BUFFER_EA_AT as u32);
    p.il(9, 0);
    p.wrch(MFC_EAH, 9);
    p.il(4, DATA_AT);
    p.ila(5, CHUNK);
    p.il(15, TAG);
    p.il(6, MFC_PUT);
    for k in 0..CHUNKS {
        p.ila(19, k * CHUNK);
        p.a(12, 11, 19);
        p.wrch(MFC_LSA, 4);
        p.wrch(MFC_EAL, 12);
        p.wrch(MFC_SIZE, 5);
        p.wrch(MFC_TAG_ID, 15);
        p.wrch(MFC_CMD, 6);
    }
    p.put(0x2000);
    p.0
}

/// True while a transfer `unit` queued under `TAG` has not landed.
fn tag_pending(rt: &Runtime, unit: UnitId) -> bool {
    rt.dma_queue().pending().any(|(c, _)| {
        c.issuer() == unit
            && c.request()
                .tag_id()
                .is_some_and(|t| u32::from(t.raw()) == TAG)
    })
}

fn run() -> Answers {
    let mut rt = Runtime::new(GuestMemory::new(0x2_0000), Budget::new(1), 2000);
    let unit = rt.register_unit_with(|id: UnitId| {
        let mut spu = SpuExecutionUnit::new(id);
        let ls = &mut spu.state_mut().ls;
        for (i, w) in program().into_iter().enumerate() {
            ls[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        for at in (DATA_AT..DATA_AT + CHUNK).step_by(4) {
            let at = at as usize;
            ls[at..at + 4].copy_from_slice(&DATA_WORD.to_be_bytes());
        }
        ls[BUFFER_EA_AT..BUFFER_EA_AT + 4].copy_from_slice(&BUFFER_EA.to_be_bytes());
        spu
    });
    let mut finished = false;
    for _ in 0..2000 {
        let Ok(step) = rt.step() else { break };
        finished = step.result.yield_reason == YieldReason::Finished;
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
        if finished {
            break;
        }
    }
    assert!(finished, "the SPU stops");
    assert!(
        tag_pending(&rt, unit),
        "the SPU stopped with its puts still queued"
    );
    // Nothing is left to step, so the run ends here and lands what the
    // MFC still holds, as the scenario runner does at termination.
    assert!(matches!(rt.step(), Err(StepError::NoRunnableUnit)));
    rt.drain_pending_dma();
    assert_eq!(rt.take_mfc_exception(), None);
    let spu = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("an SPU unit");
    let stop = spu.stop_registers().expect("a stopped SPU");
    let tag = if tag_pending(&rt, unit) { 0 } else { 1 << TAG };
    let buffer = |offset: u32| {
        let range =
            ByteRange::new(GuestAddr::new(u64::from(BUFFER_EA + offset)), 4).expect("in range");
        let bytes = rt.memory().read(range).expect("readable");
        u32::from_be_bytes(bytes.try_into().expect("4"))
    };
    let last = (0..CHUNKS).map(|k| buffer((k + 1) * CHUNK - 4)).collect();
    let first = (0..CHUNKS).map(|k| buffer(k * CHUNK)).collect();
    (stop.status, stop.npc, tag, last, first)
}

/// The MFC finishes the puts an SPU queued before it stopped: the tag
/// group completes and every 16 KB lands in main storage.
#[test]
fn transfers_queued_before_a_stop_complete_as_on_the_console() {
    let console = console();
    let (_, _, tag, last, first) = &console;
    assert_eq!(*tag, 1 << TAG, "tag 1 completes after the stop");
    assert!(
        last.iter().chain(first).all(|&w| w == DATA_WORD),
        "every put landed"
    );
    assert_eq!(run(), console);
}
