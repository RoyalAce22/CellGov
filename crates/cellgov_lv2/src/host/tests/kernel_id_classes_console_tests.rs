//! The `lv2_kernel_id_classes` probe held to its console capture: the
//! ids a mutex, a cond, an event port, an lwmutex and an lwcond take,
//! every destroy's answer, and the id a mutex takes once all five are
//! destroyed.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::kernel_id;

use super::*;
use crate::host::test_support::{
    console_payload, extract_write_u32, payload_u32, seed_primary_ppu, FakeRuntime, COND_ATTR_PTR,
    VALID_MUTEX_ATTR_PTR,
};
use crate::request::Lv2Request;
use crate::Lv2Dispatch;

const PROBE: &str = "lv2_kernel_id_classes";
const MAIN: UnitId = UnitId::new(0);

/// The probe's `created`: the return code and the written id.
fn create(host: &mut Lv2Host, rt: &FakeRuntime, request: Lv2Request) -> [u32; 2] {
    match host.dispatch(request, MAIN, rt) {
        Lv2Dispatch::Immediate { code: 0, effects } => [0, extract_write_u32(&effects[0])],
        other => panic!("{request:?}: expected a created id, got {other:?}"),
    }
}

fn rc(host: &mut Lv2Host, rt: &FakeRuntime, request: Lv2Request) -> u32 {
    match host.dispatch(request, MAIN, rt) {
        Lv2Dispatch::Immediate { code, .. } => code as u32,
        other => panic!("{request:?}: expected Immediate, got {other:?}"),
    }
}

/// The console's process already held the slots below its first
/// create's when the probe ran; the test fills them first.
#[test]
fn every_create_mints_the_consoles_id_and_a_freed_slot_comes_back_one_generation_up() {
    let p = console_payload(PROBE);
    let word = |i: usize| payload_u32(&p, 4 * i);
    assert_eq!(word(18), 0, "the console ran every step");
    assert_eq!(word(19), 0xC0DE_C1A5, "the console wrote the whole frame");

    let rt = FakeRuntime::new(0x10000).with_valid_mutex_attr();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, MAIN);
    for _ in 0..kernel_id::slot_of(word(1)) {
        host.alloc_id(cellgov_event::UnitId::new(0), 0x23)
            .expect("a free slot");
    }
    let mut ours = [0u32; 18];

    let mutex_create = Lv2Request::MutexCreate {
        id_ptr: 0x100,
        attr_ptr: VALID_MUTEX_ATTR_PTR,
    };
    [ours[0], ours[1]] = create(&mut host, &rt, mutex_create);
    let mutex = ours[1];
    [ours[2], ours[3]] = create(
        &mut host,
        &rt,
        Lv2Request::CondCreate {
            id_ptr: 0x100,
            mutex_id: mutex,
            attr_ptr: COND_ATTR_PTR,
        },
    );
    [ours[4], ours[5]] = create(
        &mut host,
        &rt,
        Lv2Request::EventPortCreate {
            id_ptr: 0x100,
            port_type: 1,
            name: 0x1234,
        },
    );
    [ours[6], ours[7]] = create(
        &mut host,
        &rt,
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
    );
    [ours[8], ours[9]] = create(
        &mut host,
        &rt,
        Lv2Request::LwCondCreate {
            id_ptr: 0x100,
            lwmutex_id: ours[7],
            control_ptr: 0x300,
            name: 0,
        },
    );

    ours[10] = rc(&mut host, &rt, Lv2Request::LwCondDestroy { id: ours[9] });
    ours[11] = rc(&mut host, &rt, Lv2Request::LwMutexDestroy { id: ours[7] });
    ours[12] = rc(&mut host, &rt, Lv2Request::EventPortDestroy { id: ours[5] });
    ours[13] = rc(&mut host, &rt, Lv2Request::CondDestroy { id: ours[3] });
    ours[14] = rc(&mut host, &rt, Lv2Request::MutexDestroy { mutex_id: mutex });

    [ours[15], ours[16]] = create(&mut host, &rt, mutex_create);
    ours[17] = rc(
        &mut host,
        &rt,
        Lv2Request::MutexDestroy { mutex_id: ours[16] },
    );

    for (i, ours) in ours.into_iter().enumerate() {
        assert_eq!(ours, word(i), "word {i}");
    }
}
