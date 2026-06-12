//! LV2 dispatch tests -- PendingResponse variant tagging, payload round-trips, and block-reason mapping.

use super::*;

#[test]
fn pending_response_variant_tags_are_distinct() {
    let tags = [
        PendingResponse::ReturnCode { code: 0 }.variant_tag(),
        PendingResponse::ThreadGroupJoin {
            group_id: 0,
            code: 0,
            cause_ptr: 0,
            status_ptr: 0,
            cause: 0,
            status: 0,
        }
        .variant_tag(),
        PendingResponse::PpuThreadJoin {
            target: 0,
            status_out_ptr: 0,
        }
        .variant_tag(),
        PendingResponse::EventQueueReceive {
            out_ptr: 0,
            payload: None,
        }
        .variant_tag(),
        PendingResponse::EventFlagWake {
            result_ptr: 0,
            observed: 0,
        }
        .variant_tag(),
        PendingResponse::CondWakeReacquire {
            mutex_id: 0,
            mutex_kind: CondMutexKind::LwMutex,
        }
        .variant_tag(),
        PendingResponse::LwMutexWake {
            mutex_ptr: 0,
            caller: 0,
        }
        .variant_tag(),
    ];
    let mut seen = std::collections::BTreeSet::new();
    for tag in tags {
        assert!(seen.insert(tag), "duplicate variant_tag byte");
    }
}

#[test]
fn event_queue_receive_payload_round_trip() {
    let original = EventPayload {
        source: 0x11,
        data1: 0x22,
        data2: 0x33,
        data3: 0x44,
    };
    let p = PendingResponse::EventQueueReceive {
        out_ptr: 0x2000,
        payload: Some(original),
    };
    match p {
        PendingResponse::EventQueueReceive { out_ptr, payload } => {
            assert_eq!(out_ptr, 0x2000);
            assert_eq!(payload, Some(original));
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn event_queue_receive_payload_none_distinct_from_some_zero() {
    let none = PendingResponse::EventQueueReceive {
        out_ptr: 0x2000,
        payload: None,
    };
    let some_zero = PendingResponse::EventQueueReceive {
        out_ptr: 0x2000,
        payload: Some(EventPayload {
            source: 0,
            data1: 0,
            data2: 0,
            data3: 0,
        }),
    };
    assert_ne!(none, some_zero);
}

#[test]
fn pending_response_cond_wake_reacquire_distinguishes_kind() {
    let lw = PendingResponse::CondWakeReacquire {
        mutex_id: 7,
        mutex_kind: CondMutexKind::LwMutex,
    };
    let hv = PendingResponse::CondWakeReacquire {
        mutex_id: 7,
        mutex_kind: CondMutexKind::Mutex,
    };
    assert_ne!(lw, hv);
}

#[test]
fn lv2_dispatch_immediate() {
    let d = Lv2Dispatch::Immediate {
        code: 0,
        effects: vec![],
    };
    assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
}

#[test]
fn spu_init_state_fields() {
    let init = SpuInitState {
        ls_bytes: vec![0; 256],
        entry_pc: 0x100,
        stack_ptr: 0x3FFF0,
        args: [1, 2, 3, 4],
        group_id: 1,
    };
    assert_eq!(init.entry_pc, 0x100);
    assert_eq!(init.args[0], 1);
}

#[test]
fn lv2_block_reason_join() {
    let r = Lv2BlockReason::ThreadGroupJoin { group_id: 5 };
    assert_eq!(r, Lv2BlockReason::ThreadGroupJoin { group_id: 5 });
}

#[test]
fn lv2_block_reason_cond_carries_mutex_kind() {
    let lw = Lv2BlockReason::Cond {
        id: 1,
        mutex_id: 7,
        mutex_kind: CondMutexKind::LwMutex,
    };
    let hv = Lv2BlockReason::Cond {
        id: 1,
        mutex_id: 7,
        mutex_kind: CondMutexKind::Mutex,
    };
    assert_ne!(lw, hv);
}
