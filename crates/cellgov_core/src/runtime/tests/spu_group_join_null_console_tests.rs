//! A group join with a NULL cause or status pointer, held to the
//! `spu_group_join_null` capture from the reference console. Each case
//! parks a join on a running one-thread group, as the probe's join
//! waited on a spinning SPU, and wakes it with the group exit the
//! probe's SPU makes; the code and the words the wake wrote are
//! compared with the captured ones.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, FakeIsaUnit, ProblemStateError,
    StopRegisters, UnitStatus,
};
use cellgov_lv2::{GroupState, Lv2Request};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::runtime::Runtime;

const PROBE: &str = "spu_group_join_null";
const CAUSE: u32 = 0x100;
const STATUS: u32 = 0x104;
/// The value the probe pre-fills both out words with.
const SENTINEL: u32 = 0xFFFF_FFFF;
/// Each case's cause and status pointers, in the probe's order.
const CASES: [(u32, u32); 4] = [(CAUSE, STATUS), (0, STATUS), (CAUSE, 0), (0, 0)];
const CASE_WORDS: usize = 8;

/// The captured payload as big-endian words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(PROBE)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// The `#define <name>` value in the probe's SPU source.
fn spu_define(name: &str) -> u32 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(PROBE)
        .join("spu/main.c");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("#define") && words.next() == Some(name))
                .then(|| words.next())
                .flatten()
        })
        .unwrap_or_else(|| panic!("no #define {name}"))
        .trim_end_matches('u')
        .parse()
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// The probe's SPU at its stop: a `sys_spu_thread_group_exit` with
/// `status` in its outbound mailbox.
#[derive(Clone)]
struct GroupExitUnit {
    inner: FakeIsaUnit,
    status: Option<u32>,
}

impl ExecutionUnit for GroupExitUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.inner.unit_id()
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.inner.run_until_yield(budget, ctx, effects)
    }

    fn snapshot(&self) {}

    fn stop_registers(&self) -> Option<StopRegisters> {
        Some(StopRegisters {
            status: (u32::from(cellgov_ps3_abi::lv2::spu::stop_code::GROUP_EXIT) << 16) | 0b10,
            npc: 0x40,
        })
    }

    fn read_out_mbox(&mut self) -> Result<Option<u32>, ProblemStateError> {
        Ok(self.status.take())
    }
}

fn word_at(rt: &Runtime, addr: u32) -> u32 {
    let mem = rt.memory().as_bytes();
    let a = addr as usize;
    u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]])
}

/// One case: a join with `cause_ptr` and `status_ptr` parked on a
/// running group whose thread then exits the group with `exit_status`.
/// Returns the code and the cause and status words after the wake.
fn join_case(cause_ptr: u32, status_ptr: u32, exit_status: u32) -> [u32; 3] {
    let mut mem = GuestMemory::new(0x1000);
    // Address 0 holds the sentinel too, so a write through a NULL
    // pointer would show.
    for addr in [0, CAUSE, STATUS] {
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(u64::from(addr)), 4).expect("a word"),
            &SENTINEL.to_be_bytes(),
        )
        .expect("the pre-fill");
    }
    let mut rt = Runtime::new(mem, Budget::new(1), 10);
    let spu = rt.register_unit_with(|id| GroupExitUnit {
        inner: FakeIsaUnit::new(id, vec![]),
        status: Some(exit_status),
    });
    let joiner = rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![]));
    let groups = rt.lv2_host_mut().thread_groups_mut();
    let gid = groups.create(1).expect("a group id");
    groups.get_mut(gid).expect("the group").state = GroupState::Running;
    groups.record_spu(spu, gid, 0).expect("the SPU joins");

    rt.dispatch_lv2_request(
        Lv2Request::SpuThreadGroupJoin {
            group_id: gid,
            cause_ptr,
            status_ptr,
        },
        joiner,
    );
    assert_eq!(
        rt.registry.effective_status(joiner),
        Some(UnitStatus::Blocked),
        "join({cause_ptr:#x}, {status_ptr:#x}) waits for the running group"
    );

    let stopped = rt.registry.get(spu).and_then(|u| u.stop_registers());
    rt.settle_finished_unit(spu, stopped);
    assert_eq!(
        rt.registry.effective_status(joiner),
        Some(UnitStatus::Runnable)
    );
    assert_eq!(word_at(&rt, 0), SENTINEL, "nothing is written through NULL");
    let code = rt
        .registry
        .drain_syscall_return(joiner)
        .expect("the wake delivers a code");
    [code as u32, word_at(&rt, CAUSE), word_at(&rt, STATUS)]
}

#[test]
fn a_join_with_a_null_cause_or_status_answers_as_on_the_console() {
    let console = console_words();
    assert_eq!((console[0], console[1]), (0, 0), "the probe ran every step");
    let exit_status = spu_define("EXIT_STATUS");

    for (k, (cause_ptr, status_ptr)) in CASES.into_iter().enumerate() {
        let w = &console[2 + CASE_WORDS * k..2 + CASE_WORDS * (k + 1)];
        // The second start succeeded although the SPU spins for a long
        // time, so the console's join waited for the group to end
        // before it answered; the follow-up join read that run's exit.
        assert_eq!(w[3], 0, "case {k}: the group had ended at the second start");
        assert_eq!((w[4], w[7]), (0, 0), "case {k}: follow-up join and destroy");

        let got = join_case(cause_ptr, status_ptr, exit_status);
        assert_eq!(got, [w[0], w[1], w[2]], "case {k}: code, cause, status");
    }
}
