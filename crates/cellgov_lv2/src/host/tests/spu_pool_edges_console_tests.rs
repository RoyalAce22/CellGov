//! Holds group create's unreadable-attribute and container answers, and
//! a finished group's hold on the SPU pool, to the `spu_pool_edges`
//! capture from the reference console. The test makes the calls of the
//! probe, in its order, through the dispatch. The fake runtime runs no
//! SPU, so the group's one thread is registered and finished through the
//! thread-group table, as the runtime does when the SPU exits.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::spu::{group_attr, group_type};

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;
use crate::thread_group::GroupState;

const PROBE: &str = "spu_pool_edges";
const CALLER: UnitId = UnitId::new(0);
const SPU: UnitId = UnitId::new(7);
const NO_SUCH_ID: u32 = 0xFFFF_FFFF;

/// One group attribute per type the probe creates, each naming
/// [`NO_SUCH_ID`] as its container.
const EXCLUSIVE: u32 = 0x800;
const CONTAINER: u32 = 0x810;
const COOP_CONTAINER: u32 = 0x820;

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
    live: Vec<u32>,
}

impl Probe {
    fn new() -> Self {
        let mut mem = GuestMemory::new(0x4000);
        for (attr, ty) in [
            (EXCLUSIVE, group_type::EXCLUSIVE_NON_CONTEXT),
            (CONTAINER, group_type::MEMORY_FROM_CONTAINER),
            (
                COOP_CONTAINER,
                group_type::COOPERATE_WITH_SYSTEM | group_type::MEMORY_FROM_CONTAINER,
            ),
        ] {
            for (offset, word) in [
                (group_attr::TYPE_OFFSET, ty),
                (group_attr::CONTAINER_OFFSET, NO_SUCH_ID),
            ] {
                mem.apply_commit(
                    ByteRange::new(GuestAddr::new(u64::from(attr + offset)), 4).unwrap(),
                    &word.to_be_bytes(),
                )
                .unwrap();
            }
        }
        Self {
            host: Lv2Host::new(),
            rt: FakeRuntime::with_memory(mem),
            live: Vec::new(),
        }
    }

    fn immediate(&mut self, request: Lv2Request) -> (u32, Vec<Effect>) {
        match self.host.dispatch(request, CALLER, &self.rt) {
            Lv2Dispatch::Immediate { code, effects } => (code as u32, effects),
            other => panic!("unexpected dispatch {other:?}"),
        }
    }

    fn initialize(&mut self, max_usable_spu: u32, max_raw_spu: u32) -> u32 {
        self.immediate(Lv2Request::SpuInitialize {
            max_usable_spu,
            max_raw_spu,
        })
        .0
    }

    /// Create a group with the attribute at `attr_ptr`; a created group
    /// is kept for `destroy_all`.
    fn create(&mut self, attr_ptr: u32, num_threads: u32) -> u32 {
        let (code, effects) = self.immediate(Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x400,
            num_threads,
            priority: 100,
            attr_ptr,
        });
        if code == 0 {
            let [Effect::SharedWriteIntent { bytes, .. }] = effects.as_slice() else {
                panic!("expected one id write, got {effects:?}");
            };
            self.live.push(u32::from_be_bytes(
                bytes.bytes().try_into().expect("4 bytes"),
            ));
        }
        code
    }

    /// Run the newest group's one thread to its exit, and return the
    /// cause a join reads.
    fn run_to_exit(&mut self) -> u32 {
        let group = *self.live.last().expect("a live group");
        let groups = &mut self.host.state.groups;
        groups.get_mut(group).expect("the group").state = GroupState::Running;
        groups.record_spu(SPU, group, 0).expect("the SPU registers");
        groups.notify_spu_finished(SPU).expect("the SPU finishes");
        groups.join_result(group).expect("a finished group").0
    }

    fn destroy_all(&mut self) {
        while let Some(id) = self.live.pop() {
            let (code, _) = self.immediate(Lv2Request::SpuThreadGroupDestroy { id });
            assert_eq!(code, 0, "destroy group {id}");
        }
    }
}

#[test]
fn group_create_edges_answer_as_the_console_does() {
    let payload = console_payload(PROBE);
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");

    let mut p = Probe::new();
    let mut got = vec![0u32; 2];
    // The fake runtime maps address 0; the probe's NULL attribute is one
    // past its arena here.
    got.push(p.create(0x4000, 1));
    got.push(p.create(0x4000, 0));
    got.push(p.create(CONTAINER, 1));
    got.push(p.create(COOP_CONTAINER, 2));
    p.destroy_all();

    got.push(p.initialize(6, 4));
    got.push(p.create(EXCLUSIVE, 1));
    got.push(p.run_to_exit());
    got.push(p.create(EXCLUSIVE, 1));
    p.destroy_all();

    got.push(p.initialize(6, 0));
    assert_eq!(got.len(), 11);

    for (w, &code) in got.iter().enumerate().skip(2) {
        assert_eq!(
            code,
            word(w),
            "w{w}: 0x{code:08x} vs console 0x{:08x}",
            word(w)
        );
    }
}
