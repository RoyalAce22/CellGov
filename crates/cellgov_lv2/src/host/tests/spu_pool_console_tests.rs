//! Holds the SPU pool split `sys_spu_initialize` sets, and the refusals
//! `sys_spu_thread_group_create` answers against it, to the
//! `spu_group_limits` capture from the reference console. The test
//! makes the calls of the probe, in its order, through the dispatch.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::spu::{group_attr, group_type};

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;

const PROBE: &str = "spu_group_limits";
const CALLER: UnitId = UnitId::new(0);

/// One group attribute per type the probe creates.
const CONTEXT: u32 = 0x800;
const EXCLUSIVE: u32 = 0x810;
const COOP: u32 = 0x820;
const BAD_TYPE: u32 = 0x830;

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
    live: Vec<u32>,
}

impl Probe {
    fn new() -> Self {
        let mut mem = GuestMemory::new(0x4000);
        for (attr, ty) in [
            (CONTEXT, group_type::NORMAL),
            (EXCLUSIVE, group_type::EXCLUSIVE_NON_CONTEXT),
            (COOP, group_type::COOPERATE_WITH_SYSTEM),
            (BAD_TYPE, 0x01),
        ] {
            let at = u64::from(attr + group_attr::TYPE_OFFSET);
            mem.apply_commit(
                ByteRange::new(GuestAddr::new(at), 4).unwrap(),
                &ty.to_be_bytes(),
            )
            .unwrap();
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

    /// Create a group; a created group is kept for `destroy_all`.
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

    fn destroy_all(&mut self) {
        while let Some(id) = self.live.pop() {
            let (code, _) = self.immediate(Lv2Request::SpuThreadGroupDestroy { id });
            assert_eq!(code, 0, "destroy group {id}");
        }
    }
}

#[test]
fn the_pool_split_and_group_create_answer_as_the_console_does() {
    let payload = console_payload(PROBE);
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");

    let mut p = Probe::new();
    let mut got = vec![0u32; 2];
    got.push(p.initialize(6, 6));
    got.push(p.initialize(0, 0));
    got.push(p.create(CONTEXT, 6));
    got.push(p.create(CONTEXT, 6));
    got.push(p.create(CONTEXT, 7));
    got.push(p.create(EXCLUSIVE, 5));
    got.push(p.create(EXCLUSIVE, 1));
    got.push(p.initialize(6, 1));
    p.destroy_all();

    got.push(p.create(EXCLUSIVE, 6));
    got.push(p.create(EXCLUSIVE, 7));
    got.push(p.initialize(6, 1));
    got.push(p.create(CONTEXT, 6));
    got.push(p.create(CONTEXT, 5));
    got.push(p.create(EXCLUSIVE, 4));
    got.push(p.create(EXCLUSIVE, 1));
    got.push(p.initialize(6, 2));
    got.push(p.initialize(6, 0));
    p.destroy_all();

    got.push(p.create(COOP, 1));
    got.push(p.create(COOP, 2));
    got.push(p.create(COOP, 2));
    p.destroy_all();

    got.push(p.initialize(6, 0));
    got.push(p.initialize(6, 1));
    got.push(p.create(EXCLUSIVE, 6));
    p.destroy_all();

    got.push(p.initialize(6, 0));
    got.push(p.create(COOP, 7));
    got.push(p.create(BAD_TYPE, 1));
    p.destroy_all();

    got.push(p.initialize(6, 0));
    assert_eq!(got.len(), 29);

    for (w, &code) in got.iter().enumerate().skip(2) {
        assert_eq!(
            code,
            word(w),
            "w{w}: 0x{code:08x} vs console 0x{:08x}",
            word(w)
        );
    }
}
