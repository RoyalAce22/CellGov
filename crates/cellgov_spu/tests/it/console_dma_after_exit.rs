//! Transfers queued before a thread exit or a group exit, held to the
//! `spu_dma_after_exit` and `spu_dma_after_group_exit` probes' committed
//! captures: an SPU thread queues sixteen 16 KB puts under one tag and
//! exits without waiting for the tag, and the group's joiner reads the
//! buffer as its join returns.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::{FakeIsaUnit, UnitStatus};
use cellgov_lv2::{GroupState, PendingResponse};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_EAH, MFC_EAL, MFC_LSA, MFC_PUT, MFC_SIZE, MFC_TAG_ID, SPU_WR_OUT_MBOX,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const THREAD_EXIT: &str =
    "../../tests/micro/spu_dma_after_exit/ps3/cech20-cex-493/observation.json";
const GROUP_EXIT: &str =
    "../../tests/micro/spu_dma_after_group_exit/ps3/cech20-cex-493/observation.json";

const CHUNK: u32 = 0x4000;
const CHUNKS: u32 = 16;
const DATA_AT: u32 = 0x4000;
const DATA_WORD: u32 = 0x5A5A_5A5A;
/// Where the buffer sits in CellGov's guest memory.
const BUFFER_EA: u32 = 0x1_0000;
const TAG: u32 = 1;
/// Where the join writes its cause and status.
const CAUSE_AT: u32 = 0x100;
const STATUS_AT: u32 = 0x104;

/// One run's answers: (join cause, join status, the first and the last
/// word of each 16 KB of the buffer as the join returns).
type Answers = (u32, u32, Vec<u32>, Vec<u32>);

fn console(path: &str) -> Answers {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    let words = |at: usize| (0..CHUNKS as usize).map(|i| word(at + i * 4)).collect();
    (word(8), word(12), words(16), words(80))
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
    fn rotqbyi(&mut self, rt: u32, ra: u32, imm: u32) {
        self.put(0x3F80_0000 | (imm & 0x7F) << 14 | ra << 7 | rt);
    }
    fn wrch(&mut self, channel: u8, rt: u32) {
        self.put(0x21A0_0000 | u32::from(channel) << 7 | rt);
    }
}

/// The probe's SPU program, word by word from address 0, ending in
/// `stop exit_code`.
fn program(exit_code: u32) -> Vec<u32> {
    let mut p = Program::default();
    p.wrch(MFC_EAH, 3);
    p.rotqbyi(11, 3, 4);
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
    p.il(2, 0);
    p.wrch(SPU_WR_OUT_MBOX, 2);
    p.put(exit_code);
    p.0
}

fn word_at(rt: &Runtime, addr: u32) -> u32 {
    let range = ByteRange::new(GuestAddr::new(u64::from(addr)), 4).expect("in range");
    let bytes = rt.memory().read(range).expect("readable");
    u32::from_be_bytes(bytes.try_into().expect("4"))
}

fn run(exit_code: u32) -> Answers {
    let mut rt = Runtime::new(GuestMemory::new(0x5_0000), Budget::new(1), 2000);
    let spu = rt.register_unit_with(|id: UnitId| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        for (i, w) in program(exit_code).into_iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        for at in (DATA_AT..DATA_AT + CHUNK).step_by(4) {
            let at = at as usize;
            state.ls[at..at + 4].copy_from_slice(&DATA_WORD.to_be_bytes());
        }
        // The thread's first argument, a doubleword in the preferred
        // slot of $3.
        state.set_reg_word_slot(3, 1, BUFFER_EA);
        spu
    });
    let joiner = rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![]));
    let groups = rt.lv2_host_mut().thread_groups_mut();
    let gid = groups.create(1).expect("a group id");
    groups.get_mut(gid).expect("the group").state = GroupState::Running;
    groups.record_spu(spu, gid, 0).expect("the SPU joins");
    rt.set_unit_status_override(joiner, UnitStatus::Blocked);
    let _ = rt.syscall_responses_mut().insert(
        joiner,
        PendingResponse::ThreadGroupJoin {
            group_id: gid,
            code: 0,
            cause_ptr: CAUSE_AT,
            status_ptr: STATUS_AT,
            cause: 0,
            status: 0,
        },
    );
    for _ in 0..2000 {
        let mut step = rt.step().expect("a unit to step");
        if step.unit == joiner {
            // The join has returned: what the joiner reads now is what
            // its first instruction after the syscall would read.
            let first = (0..CHUNKS)
                .map(|k| word_at(&rt, BUFFER_EA + k * CHUNK))
                .collect();
            let last = (0..CHUNKS)
                .map(|k| word_at(&rt, BUFFER_EA + (k + 1) * CHUNK - 4))
                .collect();
            return (word_at(&rt, CAUSE_AT), word_at(&rt, STATUS_AT), first, last);
        }
        rt.commit_step(&mut step).expect("the step commits");
    }
    panic!("the join never returned");
}

fn holds_to_the_console(path: &str, exit_code: u32) {
    let console = console(path);
    let (_, _, first, last) = &console;
    assert!(
        first.iter().chain(last).all(|&w| w == DATA_WORD),
        "every put landed before the join returned"
    );
    assert_eq!(run(exit_code), console);
}

/// A thread exit does not cut off the transfers the thread queued: by
/// the time the group's join returns, every 16 KB has landed.
#[test]
fn transfers_queued_before_a_thread_exit_land_before_the_join_returns() {
    // stop 0x102: sys_spu_thread_exit.
    holds_to_the_console(THREAD_EXIT, 0x102);
}

/// A group exit does not cut off the transfers the exiting thread
/// queued either.
#[test]
fn transfers_queued_before_a_group_exit_land_before_the_join_returns() {
    // stop 0x101: sys_spu_thread_group_exit.
    holds_to_the_console(GROUP_EXIT, 0x101);
}
