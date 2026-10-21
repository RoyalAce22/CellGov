//! A wake owed to a thread reaches the module_start unit aliased to it.

use crate::host::test_support::{
    extract_write_u32, fake_runtime_with_valid_sync_attr, seed_primary_ppu, VALID_SYNC_ATTR_PTR,
};
use crate::host::Lv2Host;
use crate::ppu_thread::PpuThreadAttrs;
use crate::request::Lv2Request;
use crate::Lv2Dispatch;
use cellgov_event::UnitId;

#[test]
fn semaphore_post_wakes_the_module_start_unit_parked_as_the_primary() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let primary_unit = UnitId::new(0);
    let poster_unit = UnitId::new(1);
    let module_start_unit = UnitId::new(9);
    seed_primary_ppu(&mut host, primary_unit);
    host.ppu_threads_mut()
        .create(
            poster_unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    assert!(host.alias_unit_to_thread_of(module_start_unit, primary_unit));
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            initial: 0,
            max: 10,
        },
        module_start_unit,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(
        Lv2Request::SemaphoreWait { id, timeout: 0 },
        module_start_unit,
        &rt,
    );
    let post = host.dispatch(Lv2Request::SemaphorePost { id, val: 1 }, poster_unit, &rt);
    match post {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            ..
        } => assert_eq!(woken_unit_ids, vec![module_start_unit]),
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
}
