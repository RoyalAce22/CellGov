//! LV2 event-flag dispatch tests: attribute validation, mode/mask matching, FIFO wakes on set, and trywait EBUSY.

use super::*;
use crate::host::test_support::{
    extract_write_u32, fake_runtime_with_valid_sync_attr, seed_primary_ppu, FakeRuntime,
    VALID_SYNC_ATTR_PTR,
};
use crate::ppu_thread::PpuThreadAttrs;
use crate::request::Lv2Request;

#[test]
fn event_flag_create_null_id_ptr_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0,
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_EFAULT.into());
    assert!(host.event_flags().is_empty());
}

#[test]
fn event_flag_create_null_attr_ptr_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            init: 0,
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_EFAULT.into());
}

#[test]
fn event_flag_create_zeroed_attr_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: 0x800,
            init: 0,
        },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_EINVAL.into());
}

#[test]
fn event_flag_create_stores_init_bits() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0x1234,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    assert_eq!(host.event_flags().lookup(id).unwrap().bits(), 0x1234);
}

#[test]
fn event_flag_wait_and_mode_mask_match_returns_observed_bits() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0b1111,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let w = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b0011,
            mode: 0x01,
            result_ptr: 0x200,
            timeout: 0,
        },
        src,
        &rt,
    );
    match w {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => {
            assert_eq!(e.len(), 1);
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    }
    assert_eq!(host.event_flags().lookup(id).unwrap().bits(), 0b1111);
}

#[test]
fn event_flag_wait_no_match_parks_caller() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let w = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b0010,
            mode: 0x01, // AND + NO-CLEAR
            result_ptr: 0x200,
            timeout: 0,
        },
        src,
        &rt,
    );
    match w {
        Lv2Dispatch::Block {
            reason: crate::dispatch::Lv2BlockReason::EventFlag { id: fid },
            pending:
                PendingResponse::EventFlagWake {
                    result_ptr,
                    observed,
                },
            ..
        } => {
            assert_eq!(fid, id);
            assert_eq!(result_ptr, 0x200);
            assert_eq!(observed, 0);
        }
        other => panic!("expected Block on EventFlag, got {other:?}"),
    }
    assert_eq!(host.event_flags().lookup(id).unwrap().waiters().len(), 1);
}

#[test]
fn event_flag_set_wakes_matching_waiters_in_fifo_order() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let u1 = UnitId::new(0);
    let u2 = UnitId::new(1);
    let u3 = UnitId::new(2);
    seed_primary_ppu(&mut host, u1);
    let _t2 = host
        .ppu_threads_mut()
        .create(
            u2,
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
    let t3 = host
        .ppu_threads_mut()
        .create(
            u3,
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
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0,
        },
        u1,
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
        Lv2Request::EventFlagWait {
            id,
            bits: 0b0001,
            mode: 0x01,
            result_ptr: 0x200,
            timeout: 0,
        },
        u1,
        &rt,
    );
    host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b0010,
            mode: 0x01,
            result_ptr: 0x210,
            timeout: 0,
        },
        u2,
        &rt,
    );
    host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b1000,
            mode: 0x01,
            result_ptr: 0x220,
            timeout: 0,
        },
        u3,
        &rt,
    );
    let s = host.dispatch(Lv2Request::EventFlagSet { id, bits: 0b0011 }, u1, &rt);
    match s {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            response_updates,
            ..
        } => {
            assert_eq!(woken_unit_ids, vec![u1, u2]);
            assert_eq!(response_updates.len(), 2);
            for (_, resp) in &response_updates {
                match resp {
                    PendingResponse::EventFlagWake { observed, .. } => {
                        assert_eq!(*observed, 0b0011);
                    }
                    other => panic!("expected EventFlagWake, got {other:?}"),
                }
            }
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    let remaining: Vec<_> = host
        .event_flags()
        .lookup(id)
        .unwrap()
        .waiters()
        .iter()
        .map(|w| w.thread)
        .collect();
    assert_eq!(remaining, vec![t3]);
}

#[test]
fn event_flag_clear_does_not_wake_anyone() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0b1111,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let c = host.dispatch(Lv2Request::EventFlagClear { id, bits: 0b0101 }, src, &rt);
    assert!(matches!(
        c,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    // sys_event_flag_clear masks AND: 0b1111 & 0b0101 -> 0b0101.
    assert_eq!(host.event_flags().lookup(id).unwrap().bits(), 0b0101);
}

#[test]
fn event_flag_trywait_no_match_returns_ebusy() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let w = host.dispatch(
        Lv2Request::EventFlagTryWait {
            id,
            bits: 0b1,
            mode: 0x01,
            result_ptr: 0x200,
        },
        src,
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = w else {
        panic!("expected Immediate, got {w:?}");
    };
    assert_eq!(code, cell_errors::CELL_EBUSY.into());
}
