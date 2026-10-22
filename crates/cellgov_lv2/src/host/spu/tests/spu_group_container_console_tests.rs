//! Holds the memory a container-type SPU thread group takes from its
//! container to the `spu_group_container`, `spu_group_container_shared`
//! and `spu_group_container_coop` captures from the reference console.
//! Each test makes the calls of its probe, in its order, through the
//! dispatch.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::spu::{group_attr, group_type};

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;

const CALLER: UnitId = UnitId::new(0);
const ATTR: u32 = 0x800;
const OUT: u32 = 0x400;
const NO_SUCH_CONTAINER: u32 = 0x7FFF_FFF0;

struct Probe {
    host: Lv2Host,
}

impl Probe {
    /// Dispatch `request` against a runtime whose group attribute names
    /// `group_type` and `container`.
    fn immediate(&mut self, request: Lv2Request, attr: (u32, u32)) -> (u32, Vec<Effect>) {
        let mut mem = GuestMemory::new(0x1000);
        for (offset, word) in [
            (group_attr::TYPE_OFFSET, attr.0),
            (group_attr::CONTAINER_OFFSET, attr.1),
        ] {
            mem.apply_commit(
                ByteRange::new(GuestAddr::new(u64::from(ATTR + offset)), 4).unwrap(),
                &word.to_be_bytes(),
            )
            .unwrap();
        }
        let rt = FakeRuntime::with_memory(mem);
        match self.host.dispatch(request, CALLER, &rt) {
            Lv2Dispatch::Immediate { code, effects } => (code as u32, effects),
            other => panic!("unexpected dispatch {other:?}"),
        }
    }

    fn written(effects: &[Effect]) -> u32 {
        let [Effect::SharedWriteIntent { bytes, .. }] = effects else {
            panic!("expected one out-word write, got {effects:?}");
        };
        u32::from_be_bytes(bytes.bytes().try_into().expect("4 bytes"))
    }

    fn initialize(&mut self) -> u32 {
        let request = Lv2Request::SpuInitialize {
            max_usable_spu: 6,
            max_raw_spu: 0,
        };
        self.immediate(request, (0, 0)).0
    }

    /// The return code, and the id a successful create wrote.
    fn container_create(&mut self, size: u64) -> (u32, Option<u32>) {
        let request = Lv2Request::MemoryContainerCreate { cid_ptr: OUT, size };
        let (code, effects) = self.immediate(request, (0, 0));
        (code, (code == 0).then(|| Self::written(&effects)))
    }

    /// The return code, and the id a successful create wrote.
    fn create(&mut self, ty: u32, container: u32, num_threads: u32) -> (u32, Option<u32>) {
        let request = Lv2Request::SpuThreadGroupCreate {
            id_ptr: OUT,
            num_threads,
            priority: 100,
            attr_ptr: ATTR,
        };
        let (code, effects) = self.immediate(request, (ty, container));
        (code, (code == 0).then(|| Self::written(&effects)))
    }

    /// The destroy's return code, or the probe's 0xEEEEEEEE when the
    /// create it follows made no group.
    fn destroy(&mut self, id: Option<u32>) -> u32 {
        id.map_or(0xEEEE_EEEE, |id| {
            self.immediate(Lv2Request::SpuThreadGroupDestroy { id }, (0, 0))
                .0
        })
    }

    fn allocate_from_container(&mut self, size: u64, cid: u32, flags: u64) -> u32 {
        let request = Lv2Request::MemoryAllocateFromContainer {
            size,
            cid,
            flags,
            alloc_addr_ptr: OUT,
        };
        self.immediate(request, (0, 0)).0
    }

    /// Destroy a group a refused step may have left behind.
    fn drop_group(&mut self, id: Option<u32>) {
        if id.is_some() {
            self.destroy(id);
        }
    }
}

#[test]
fn a_container_group_takes_its_memory_as_the_console_does() {
    let payload = console_payload("spu_group_container");
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");

    const CONTAINER: u32 = group_type::MEMORY_FROM_CONTAINER;
    let mut p = Probe {
        host: Lv2Host::new(),
    };
    let mut got = vec![(0, 0), (1, 0)];
    let mut put = |w: usize, code: u32| got.push((w, code));
    put(2, p.initialize());
    let (code, a) = p.container_create(0x10_0000);
    put(3, code);
    let (code, b) = p.container_create(0x18_0000);
    put(5, code);
    let (a, b) = (a.unwrap(), b.unwrap());

    let (code, tmp) = p.create(CONTAINER, b, 5);
    put(7, code);
    p.drop_group(tmp);
    let (code, tmp) = p.create(CONTAINER, b, 4);
    put(8, code);
    put(9, p.destroy(tmp));

    let (code, in_a) = p.create(CONTAINER, a, 4);
    put(10, code);
    let (code, tmp) = p.create(CONTAINER, a, 1);
    put(11, code);
    p.drop_group(tmp);
    let (code, normal) = p.create(group_type::NORMAL, 0, 2);
    put(12, code);
    let (code, tmp) = p.create(CONTAINER, a, 1);
    put(13, code);
    p.drop_group(tmp);
    let (code, tmp) = p.create(CONTAINER, b, 1);
    put(14, code);
    p.drop_group(tmp);

    put(15, p.destroy(in_a));
    let (code, tmp) = p.create(CONTAINER, a, 1);
    put(16, code);
    put(17, p.destroy(tmp));
    put(18, p.destroy(normal));

    let (code, tmp) = p.create(CONTAINER, NO_SUCH_CONTAINER, 1);
    put(19, code);
    p.drop_group(tmp);
    put(20, p.container_create(0x8_0000).0);
    put(21, p.initialize());
    assert_eq!(
        got.len(),
        20,
        "w4 and w6 are container ids the console mints"
    );

    for (w, code) in got {
        assert_eq!(
            code,
            word(w),
            "w{w}: 0x{code:08x} vs console 0x{:08x}",
            word(w)
        );
    }
}

/// Assert each `(word, code)` against the payload of `probe`.
fn assert_matches_console(probe: &str, got: &[(usize, u32)]) {
    let payload = console_payload(probe);
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");
    for &(w, code) in got {
        assert_eq!(
            code,
            word(w),
            "{probe} w{w}: 0x{code:08x} vs console 0x{:08x}",
            word(w)
        );
    }
}

#[test]
fn a_cooperating_group_and_a_container_allocation_take_memory_as_the_console_does() {
    const CONTAINER: u32 = group_type::MEMORY_FROM_CONTAINER;
    const COOP: u32 = group_type::COOPERATE_WITH_SYSTEM | CONTAINER;
    let mut p = Probe {
        host: Lv2Host::new(),
    };
    let mut got = Vec::new();
    got.push((2, p.initialize()));
    let (code, a) = p.container_create(0x10_0000);
    got.push((3, code));
    let a = a.unwrap();
    let (code, coop) = p.create(COOP, a, 4);
    got.push((5, code));
    let (code, tmp) = p.create(CONTAINER, a, 3);
    got.push((6, code));
    got.push((7, p.destroy(tmp)));
    got.push((8, p.destroy(coop)));

    let (code, c) = p.container_create(0x10_0000);
    got.push((9, code));
    let c = c.unwrap();
    got.push((11, p.allocate_from_container(0x10_0000, c, 0x400)));
    let (code, tmp) = p.create(CONTAINER, c, 1);
    got.push((13, code));
    got.push((14, p.destroy(tmp)));
    got.push((15, p.initialize()));
    // w4 and w10 are container ids and w12 an address the console mints.
    assert_matches_console("spu_group_container_shared", &got);
}

#[test]
fn a_cooperating_group_takes_one_local_store_as_the_console_does() {
    const CONTAINER: u32 = group_type::MEMORY_FROM_CONTAINER;
    const COOP: u32 = group_type::COOPERATE_WITH_SYSTEM | CONTAINER;
    let mut p = Probe {
        host: Lv2Host::new(),
    };
    let mut got = Vec::new();
    got.push((2, p.initialize()));
    let (code, a) = p.container_create(0x10_0000);
    got.push((3, code));
    let a = a.unwrap();
    let (code, coop) = p.create(COOP, a, 2);
    got.push((5, code));
    let (code, tmp) = p.create(CONTAINER, a, 4);
    got.push((6, code));
    got.push((7, p.destroy(tmp)));
    let (code, tmp) = p.create(CONTAINER, a, 3);
    got.push((8, code));
    got.push((9, p.destroy(tmp)));
    got.push((10, p.destroy(coop)));
    got.push((11, p.initialize()));
    // w4 is a container id the console mints.
    assert_matches_console("spu_group_container_coop", &got);
}
