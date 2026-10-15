//! Holds group suspend, resume and yield, a terminate of a suspended
//! group, a mailbox write to a suspended thread, a start of a finished
//! group with and without a join, and the exit status a restarted thread
//! reports to the `spu_group_suspend` capture from the reference console.
//! The test makes the calls of the probe, in its order, through the
//! dispatch. The fake runtime runs no SPU: group A's thread only waits
//! for the terminate, and the test reports group B's exit as the runtime
//! does.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::spu::{group_attr, group_type};

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;

const PROBE: &str = "spu_group_suspend";
const CALLER: UnitId = UnitId::new(0);
const NO_SUCH_ID: u32 = 0xFFFF_FFFF;
const IMG_PTR: u32 = 0x300;
/// The SELF control flags every probe EBOOT carries: root permission.
const PROBE_CONTROL_FLAGS1: u32 = 0x4000_0000;
/// The messages the probe writes to group B's thread, one per run; the
/// first lands while the group is suspended.
const MESSAGE: u32 = 0x5A;
const MESSAGE_2: u32 = 0x6B;
const MESSAGE_3: u32 = 0x7C;
const MESSAGE_4: u32 = 0x8D;
/// The unit the runtime registers for group B's SPU in each run.
const SPU_B: UnitId = UnitId::new(9);
const SPU_B_RUN_2: UnitId = UnitId::new(10);
const SPU_B_RUN_3: UnitId = UnitId::new(11);
const SPU_B_RUN_4: UnitId = UnitId::new(12);

/// One group attribute per type the probe creates.
const CONTEXT: u32 = 0x800;
const EXCLUSIVE: u32 = 0x810;
const COOP: u32 = 0x820;

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
}

fn put(mem: &mut GuestMemory, addr: u32, bytes: &[u8]) {
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(addr)), bytes.len() as u64).unwrap(),
        bytes,
    )
    .unwrap();
}

/// The answer the guest reads in r3, and the words the call wrote.
fn answer(d: Lv2Dispatch) -> (u32, Vec<u32>) {
    let words = |effects: &[Effect]| {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::SharedWriteIntent { bytes, .. } => Some(u32::from_be_bytes(
                    bytes.bytes()[..4].try_into().expect("4 bytes"),
                )),
                _ => None,
            })
            .collect()
    };
    match d {
        Lv2Dispatch::Immediate { code, effects } => (code as u32, words(&effects)),
        Lv2Dispatch::RegisterSpu { code, .. } => (code as u32, vec![]),
        Lv2Dispatch::SpuGroupTerminate { .. } | Lv2Dispatch::SpuGroupSuspension { .. } => {
            (0, vec![])
        }
        other => panic!("unexpected dispatch {other:?}"),
    }
}

impl Probe {
    fn new() -> Self {
        let mut mem = GuestMemory::new(0x4000);
        for (attr, ty) in [
            (CONTEXT, group_type::NORMAL),
            (EXCLUSIVE, group_type::EXCLUSIVE_NON_CONTEXT),
            (COOP, group_type::COOPERATE_WITH_SYSTEM),
        ] {
            put(&mut mem, attr + group_attr::TYPE_OFFSET, &ty.to_be_bytes());
        }
        // The kernel record image open writes: type KERNEL, image id 1.
        put(&mut mem, IMG_PTR, &[0, 0, 0, 1, 0, 0, 0, 1]);
        let mut host = Lv2Host::new();
        host.set_control_flags1(PROBE_CONTROL_FLAGS1);
        host.content_store_mut().register(b"/spu.elf", vec![0xAA]);
        Self {
            host,
            rt: FakeRuntime::with_memory(mem),
        }
    }

    fn call(&mut self, request: Lv2Request) -> (u32, Vec<u32>) {
        answer(self.host.dispatch(request, CALLER, &self.rt))
    }

    fn create(&mut self, attr_ptr: u32, num_threads: u32) -> u32 {
        let (code, words) = self.call(Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x400,
            num_threads,
            priority: 100,
            attr_ptr,
        });
        assert_eq!(code, 0, "create");
        words[0]
    }

    fn suspend(&mut self, group_id: u32) -> u32 {
        self.call(Lv2Request::SpuThreadGroupSuspend { group_id }).0
    }

    fn resume(&mut self, group_id: u32) -> u32 {
        self.call(Lv2Request::SpuThreadGroupResume { group_id }).0
    }

    fn yield_(&mut self, group_id: u32) -> u32 {
        self.call(Lv2Request::SpuThreadGroupYield { group_id }).0
    }

    fn start(&mut self, group_id: u32) -> u32 {
        self.call(Lv2Request::SpuThreadGroupStart { group_id }).0
    }

    fn terminate(&mut self, group_id: u32, value: i32) -> u32 {
        self.call(Lv2Request::SpuThreadGroupTerminate { group_id, value })
            .0
    }

    /// Join; returns the cause and status the join wrote.
    fn join(&mut self, group_id: u32) -> [u32; 2] {
        let (code, words) = self.call(Lv2Request::SpuThreadGroupJoin {
            group_id,
            cause_ptr: 0x600,
            status_ptr: 0x604,
        });
        assert_eq!(code, 0, "join");
        [words[0], words[1]]
    }

    /// `sys_spu_thread_write_spu_mb`'s answer.
    fn write_mb(&mut self, thread_id: u32, value: u32) -> u32 {
        self.call(Lv2Request::SpuThreadWriteMb { thread_id, value })
            .0
    }

    /// `sys_spu_thread_get_exit_status`'s answer, and the status it
    /// wrote (0xFFFFFFFF when it wrote none, as the probe pre-fills).
    fn exit_status(&mut self, thread_id: u32) -> [u32; 2] {
        let (code, words) = self.call(Lv2Request::SpuThreadGetExitStatus {
            thread_id,
            status_ptr: 0x608,
        });
        [code, words.first().copied().unwrap_or(0xFFFF_FFFF)]
    }

    /// The runtime registers the SPU a start of `group_id` created.
    fn register_spu(&mut self, unit: UnitId, group_id: u32) {
        self.host
            .thread_groups_mut()
            .record_spu(unit, group_id, 0)
            .expect("the runtime registers the SPU");
    }

    /// The SPU `unit` read `message` from its mailbox and exited with it.
    fn exit_thread(&mut self, unit: UnitId, message: u32) {
        self.host
            .spu_thread_exit(unit, message)
            .expect("the thread exits with the message it read");
    }
}

#[test]
fn suspend_resume_yield_and_restart_answer_as_the_console_does() {
    let payload = console_payload(PROBE);
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");

    let mut p = Probe::new();
    let a = p.create(CONTEXT, 1);
    let (code, _) = p.call(Lv2Request::SpuThreadInitialize {
        thread_ptr: 0x500,
        group_id: a,
        thread_num: 0,
        img_ptr: IMG_PTR,
        attr_ptr: 0,
        arg_ptr: 0,
    });
    assert_eq!(code, 0, "initialize");

    let mut got = vec![0u32; 2];
    got.extend([
        p.suspend(NO_SUCH_ID),
        p.resume(NO_SUCH_ID),
        p.yield_(NO_SUCH_ID),
        p.suspend(a),
        p.resume(a),
        p.yield_(a),
    ]);
    assert_eq!(p.start(a), 0);
    got.extend([
        p.yield_(a),
        p.resume(a),
        p.suspend(a),
        p.suspend(a),
        p.yield_(a),
        p.resume(a),
        p.resume(a),
        p.terminate(a, 1),
    ]);
    got.extend(p.join(a));
    got.extend([p.suspend(a), p.resume(a), p.yield_(a), p.start(a)]);
    got.extend([p.suspend(a), p.terminate(a, 2)]);
    got.extend(p.join(a));

    let x = p.create(EXCLUSIVE, 1);
    let coop = p.create(COOP, 2);
    got.extend([
        p.suspend(x),
        p.resume(x),
        p.yield_(x),
        p.suspend(coop),
        p.resume(coop),
        p.yield_(coop),
    ]);
    assert_eq!(got.len(), 32);

    // Group B: its thread reads its inbound mailbox and exits with the
    // message. The runtime registers the started SPU and reports its
    // exit; the message reaching a suspended thread is the runtime's
    // `spu_group_suspend_tests` in cellgov_core.
    let b = p.create(CONTEXT, 1);
    let (code, words) = p.call(Lv2Request::SpuThreadInitialize {
        thread_ptr: 0x500,
        group_id: b,
        thread_num: 0,
        img_ptr: IMG_PTR,
        attr_ptr: 0,
        arg_ptr: 0,
    });
    assert_eq!(code, 0, "initialize B");
    let thread = words[0];
    assert_eq!(p.start(b), 0);
    p.register_spu(SPU_B, b);
    got.push(p.suspend(b));
    got.push(p.write_mb(thread, MESSAGE));
    got.push(p.resume(b));
    p.exit_thread(SPU_B, MESSAGE);
    got.extend(p.join(b));
    got.extend(p.exit_status(thread));
    got.push(0);
    assert_eq!(got.len(), 40);

    // B again after the join: the thread keeps its exit status until it
    // exits again.
    got.push(p.start(b));
    p.register_spu(SPU_B_RUN_2, b);
    got.extend(p.exit_status(thread));
    got.push(p.write_mb(thread, MESSAGE_2));
    p.exit_thread(SPU_B_RUN_2, MESSAGE_2);
    got.extend(p.join(b));
    got.extend(p.exit_status(thread));

    // B run to its end with no join, then started again.
    assert_eq!(p.start(b), 0);
    p.register_spu(SPU_B_RUN_3, b);
    assert_eq!(p.write_mb(thread, MESSAGE_3), 0);
    p.exit_thread(SPU_B_RUN_3, MESSAGE_3);
    got.push(p.start(b));
    p.register_spu(SPU_B_RUN_4, b);
    got.push(p.write_mb(thread, MESSAGE_4));
    p.exit_thread(SPU_B_RUN_4, MESSAGE_4);
    got.extend(p.join(b));
    got.extend(p.exit_status(thread));
    got.push(0);
    got.push(p.call(Lv2Request::SpuThreadGroupDestroy { id: b }).0);
    assert_eq!(got.len(), 56);

    for (w, &value) in got.iter().enumerate().skip(2) {
        assert_eq!(
            value,
            word(w),
            "w{w}: 0x{value:08x} vs console 0x{:08x}",
            word(w)
        );
    }
}
