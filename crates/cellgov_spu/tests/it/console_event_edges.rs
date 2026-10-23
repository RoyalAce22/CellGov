//! The Qv and Sn events as the `spu_event_edges` capture shows them:
//! each follows the edge of its channel count, run through the runtime.
//!
//! A new SPU has neither event pending, though its MFC_Cmd count starts
//! at the queue depth. A second list that stalls while
//! MFC_RdListStallStat still names the first raises no Sn, and one that
//! stalls after the status was read raises it again. A get that never
//! fills the command queue raises no Qv.

// [CBEA p:149 s:9.11.1] hardware determines events by detecting the channel counts that change from 0 to a nonzero value.

use cellgov_core::Runtime;
use cellgov_exec::UnitStatus;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    event, MFC_CMD, MFC_EAL, MFC_GET, MFC_GETL, MFC_LIST_STALL_NOTIFY, MFC_LSA,
    MFC_RD_LIST_STALL_STAT, MFC_RD_TAG_STAT, MFC_SIZE, MFC_TAG_ID, MFC_TAG_UPDATE_ALL,
    MFC_WR_LIST_STALL_ACK, MFC_WR_TAG_MASK, MFC_WR_TAG_UPDATE, SPU_RD_EVENT_MASK,
    SPU_RD_EVENT_STAT, SPU_WR_EVENT_ACK, SPU_WR_EVENT_MASK,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/spu_event_edges/ps3/cech20-cex-493/observation.json";

/// The probe's tag groups: lists A and B, and the lone get.
const TAG_A: u32 = 3;
const TAG_B: u32 = 6;
const TAG_GET: u32 = 1;
const SOURCE_EA: u32 = 0x1000;
const LIST_A: u32 = 0x200;
const LIST_B: u32 = 0x210;
const DATA_A: u32 = 0x400;
const DATA_B: u32 = 0x440;
/// The SPU's poll bound for the second Sn.
const POLLS: u32 = 300;

/// The probe's events at reset: every source but Tm, Le, Me and A.
const ALL_BUT_TM_LE_ME_A: u32 =
    event::MS | event::LR | event::S1 | event::S2 | event::MB | event::QV | event::SN | event::TG;

/// Phase `phase`'s four words.
fn console_record(phase: usize) -> [u32; 4] {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let payload = &observation.memory_regions[0].data;
    assert_eq!(payload.len(), 80);
    let word = |at: usize| u32::from_be_bytes(payload[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!(word(64), 0, "no step of the probe failed");
    [0, 4, 8, 12].map(|at| word(16 * phase + at))
}

/// An SPU program, word by word from address 0.
#[derive(Default)]
struct Program(Vec<u32>);

impl Program {
    fn put(&mut self, word: u32) {
        self.0.push(word);
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
    fn il(&mut self, rt: u32, imm: u32) {
        self.put(0x4080_0000 | (imm & 0xFFFF) << 7 | rt);
    }
    fn ai(&mut self, rt: u32, ra: u32, imm: u32) {
        self.put(0x1C00_0000 | (imm & 0x3FF) << 14 | ra << 7 | rt);
    }
    /// `brz rt, words`: branch `words` instructions from this one.
    fn brz(&mut self, rt: u32, words: i32) {
        self.put(0x2000_0000 | (words as u32 & 0xFFFF) << 7 | rt);
    }
    /// `brnz rt, words`.
    fn brnz(&mut self, rt: u32, words: i32) {
        self.put(0x2100_0000 | (words as u32 & 0xFFFF) << 7 | rt);
    }

    /// The probe's `quiet()`, then `events` enabled ($3 = 0, $4 = all
    /// ones, $10 = `events`).
    fn quiet_then_enable(&mut self, events: u32) {
        self.il(3, 0);
        // il sign-extends its immediate: all ones.
        self.il(4, 0xFFFF);
        self.wrch(SPU_WR_EVENT_MASK, 3);
        self.wrch(SPU_WR_EVENT_ACK, 4);
        self.il(10, events);
        self.wrch(SPU_WR_EVENT_MASK, 10);
    }

    /// A transfer command: $2 the opcode, the channels from the
    /// registers named.
    fn command(&mut self, lsa: u32, eal: u32, size: u32, tag: u32) {
        for (channel, reg, value) in [
            (MFC_LSA, 20, lsa),
            (MFC_EAL, 21, eal),
            (MFC_SIZE, 22, size),
            (MFC_TAG_ID, 23, tag),
        ] {
            self.il(reg, value);
            self.wrch(channel, reg);
        }
        self.wrch(MFC_CMD, 2);
    }

    /// Polls the event count at most POLLS times; $13 = the status read
    /// and acknowledged, or 0 when the count never rose.
    fn poll_event(&mut self) {
        self.il(12, POLLS);
        self.rchcnt(6, SPU_RD_EVENT_STAT);
        self.brnz(6, 3);
        self.ai(12, 12, 0x3FF);
        self.brnz(12, -3);
        self.il(13, 0);
        self.brz(6, 3);
        self.rdch(13, SPU_RD_EVENT_STAT);
        self.wrch(SPU_WR_EVENT_ACK, 13);
    }

    /// Resumes both lists and waits for both groups.
    fn resume_both(&mut self) {
        for tag in [TAG_A, TAG_B] {
            self.il(24, tag);
            self.wrch(MFC_WR_LIST_STALL_ACK, 24);
        }
        self.il(24, 1 << TAG_A | 1 << TAG_B);
        self.wrch(MFC_WR_TAG_MASK, 24);
        self.il(24, MFC_TAG_UPDATE_ALL);
        self.wrch(MFC_WR_TAG_UPDATE, 24);
        self.rdch(25, MFC_RD_TAG_STAT);
    }
}

/// Runs `program` on one SPU, its two lists in local store and the
/// lists' source in guest memory, and returns registers 0 to 31.
fn run(program: &Program, opcode: u32) -> Vec<u32> {
    let mut memory = GuestMemory::new(0x2000);
    let source = ByteRange::new(GuestAddr::new(u64::from(SOURCE_EA)), 0x40).expect("in range");
    memory
        .apply_commit(source, &[0xA5; 0x40])
        .expect("writable");
    let mut rt = Runtime::new(memory, Budget::new(100), 400);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        let mut words = program.0.clone();
        words.push(0);
        for (i, word) in words.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        for list in [LIST_A, LIST_B] {
            for (i, (head, eal)) in [
                (16 | MFC_LIST_STALL_NOTIFY, SOURCE_EA),
                (16, SOURCE_EA + 16),
            ]
            .into_iter()
            .enumerate()
            {
                let at = list as usize + 8 * i;
                state.ls[at..at + 4].copy_from_slice(&head.to_be_bytes());
                state.ls[at + 4..at + 8].copy_from_slice(&eal.to_be_bytes());
            }
        }
        state.set_reg_word_splat(2, opcode);
        spu
    });
    for _ in 0..2000 {
        if rt.registry().effective_status(unit) == Some(UnitStatus::Finished) {
            break;
        }
        let Ok(mut step) = rt.step() else { break };
        rt.commit_step(&mut step).expect("the step commits");
    }
    assert_eq!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Finished),
        "the SPU ran to its stop"
    );
    let state = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit")
        .state();
    (0..32).map(|reg| state.reg_word(reg)).collect()
}

#[test]
fn a_new_spu_has_no_event_pending_as_on_the_console() {
    let console = console_record(0);
    let mut p = Program::default();
    p.rdch(7, SPU_RD_EVENT_MASK);
    p.il(10, event::QV);
    p.wrch(SPU_WR_EVENT_MASK, 10);
    p.rchcnt(5, SPU_RD_EVENT_STAT);
    p.il(10, ALL_BUT_TM_LE_ME_A);
    p.wrch(SPU_WR_EVENT_MASK, 10);
    p.rchcnt(6, SPU_RD_EVENT_STAT);
    let r = run(&p, 0);
    assert_eq!(
        (r[5], r[6], r[7]),
        (console[0], console[1], console[3]),
        "(count with Qv enabled, count with all enabled, the starting mask)"
    );
}

/// The probe's phases 1 and 2: list A stalls and Sn is acknowledged,
/// MFC_RdListStallStat read in between when `read_between`, then list B
/// stalls. Returns (the first status, the second or 0, the stall status
/// read after list B).
fn second_stall(read_between: bool) -> (u32, u32, u32) {
    let mut p = Program::default();
    p.quiet_then_enable(event::SN);
    p.command(DATA_A, LIST_A, 16, TAG_A);
    p.rdch(5, SPU_RD_EVENT_STAT);
    p.wrch(SPU_WR_EVENT_ACK, 5);
    if read_between {
        p.rdch(7, MFC_RD_LIST_STALL_STAT);
    }
    p.command(DATA_B, LIST_B, 16, TAG_B);
    p.poll_event();
    p.rdch(9, MFC_RD_LIST_STALL_STAT);
    p.resume_both();
    let r = run(&p, MFC_GETL);
    (r[5], r[13], r[9])
}

#[test]
fn a_second_stall_raises_no_sn_while_the_first_is_unread_as_on_the_console() {
    let console = console_record(1);
    assert_eq!(
        second_stall(false),
        (console[0], console[1], console[3]),
        "(first status, second status, the stall status after)"
    );
}

#[test]
fn a_second_stall_raises_sn_once_the_first_was_read_as_on_the_console() {
    let console = console_record(2);
    assert_eq!(
        second_stall(true),
        (console[0], console[1], console[3]),
        "(first status, second status, the stall status after)"
    );
}

#[test]
fn a_get_that_never_fills_the_queue_raises_no_qv_as_on_the_console() {
    let console = console_record(3);
    let mut p = Program::default();
    p.quiet_then_enable(event::QV);
    p.command(DATA_A, SOURCE_EA, 16, TAG_GET);
    p.il(24, 1 << TAG_GET);
    p.wrch(MFC_WR_TAG_MASK, 24);
    p.il(24, MFC_TAG_UPDATE_ALL);
    p.wrch(MFC_WR_TAG_UPDATE, 24);
    p.rdch(25, MFC_RD_TAG_STAT);
    p.rchcnt(5, SPU_RD_EVENT_STAT);
    p.rchcnt(6, MFC_CMD);
    let r = run(&p, MFC_GET);
    assert_eq!(
        (r[5], r[6]),
        (console[0], console[2]),
        "(event count once the get completed, MFC_Cmd count)"
    );
}
