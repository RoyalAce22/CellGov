//! Sync-state lanes of block reasons and pending responses.

use cellgov_mem::lanes::{LaneMap, LaneValue, ObjectLanes};

use crate::dispatch::{CondMutexKind, Lv2BlockReason, PendingResponse};
use crate::sync_primitives::EventPayload;

struct Reason(Lv2BlockReason);

impl LaneValue for Reason {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        assert_eq!(self.0.push_lanes(lanes, 1), 5);
    }
}

fn partial<V: LaneValue>(value: V) -> u128 {
    let mut map = LaneMap::new(1, |k: u64| k);
    map.insert(0, value);
    map.partial()
}

fn assert_all_distinct(partials: &[u128]) {
    for (i, a) in partials.iter().enumerate() {
        for (j, b) in partials.iter().enumerate().skip(i + 1) {
            assert_ne!(a, b, "entries {i} and {j} hash alike");
        }
    }
}

#[test]
fn every_block_reason_and_every_payload_field_moves_the_lanes() {
    let reasons = [
        Lv2BlockReason::ThreadGroupJoin { group_id: 1 },
        Lv2BlockReason::ThreadGroupJoin { group_id: 2 },
        Lv2BlockReason::PpuThreadJoin { target: 1 },
        Lv2BlockReason::LwMutex { id: 1 },
        Lv2BlockReason::Mutex { id: 1 },
        Lv2BlockReason::Semaphore { id: 1 },
        Lv2BlockReason::EventQueue { id: 1 },
        Lv2BlockReason::EventFlag { id: 1 },
        Lv2BlockReason::Uart,
        Lv2BlockReason::UsbdEvent { handle: 1 },
        Lv2BlockReason::Cond {
            id: 1,
            mutex_id: 2,
            mutex_kind: CondMutexKind::LwMutex,
        },
        Lv2BlockReason::Cond {
            id: 1,
            mutex_id: 2,
            mutex_kind: CondMutexKind::Mutex,
        },
        Lv2BlockReason::Cond {
            id: 1,
            mutex_id: 3,
            mutex_kind: CondMutexKind::Mutex,
        },
        Lv2BlockReason::Cond {
            id: 2,
            mutex_id: 3,
            mutex_kind: CondMutexKind::Mutex,
        },
    ];
    let partials: Vec<u128> = reasons.into_iter().map(|r| partial(Reason(r))).collect();
    assert_all_distinct(&partials);
}

#[test]
fn every_response_variant_and_every_payload_field_moves_the_lanes() {
    let join = PendingResponse::ThreadGroupJoin {
        group_id: 1,
        code: 2,
        cause_ptr: 3,
        status_ptr: 4,
        cause: 5,
        status: 6,
    };
    let PendingResponse::ThreadGroupJoin {
        group_id,
        code,
        cause_ptr,
        status_ptr,
        cause,
        status,
    } = join
    else {
        unreachable!()
    };
    let payload = EventPayload {
        source: 1,
        data1: 2,
        data2: 3,
        data3: 4,
    };
    let responses = [
        PendingResponse::ReturnCode { code: 0 },
        PendingResponse::ReturnCode { code: 1 },
        join,
        PendingResponse::ThreadGroupJoin {
            group_id: 9,
            code,
            cause_ptr,
            status_ptr,
            cause,
            status,
        },
        PendingResponse::ThreadGroupJoin {
            group_id,
            code: 9,
            cause_ptr,
            status_ptr,
            cause,
            status,
        },
        PendingResponse::ThreadGroupJoin {
            group_id,
            code,
            cause_ptr: 9,
            status_ptr,
            cause,
            status,
        },
        PendingResponse::ThreadGroupJoin {
            group_id,
            code,
            cause_ptr,
            status_ptr: 9,
            cause,
            status,
        },
        PendingResponse::ThreadGroupJoin {
            group_id,
            code,
            cause_ptr,
            status_ptr,
            cause: 9,
            status,
        },
        PendingResponse::ThreadGroupJoin {
            group_id,
            code,
            cause_ptr,
            status_ptr,
            cause,
            status: 9,
        },
        PendingResponse::PpuThreadJoin {
            target: 1,
            status_out_ptr: 2,
        },
        PendingResponse::PpuThreadJoin {
            target: 1,
            status_out_ptr: 3,
        },
        PendingResponse::EventQueueReceive {
            out_ptr: 1,
            payload: None,
        },
        PendingResponse::EventQueueReceive {
            out_ptr: 1,
            payload: Some(EventPayload {
                source: 0,
                data1: 0,
                data2: 0,
                data3: 0,
            }),
        },
        PendingResponse::EventQueueReceive {
            out_ptr: 1,
            payload: Some(payload),
        },
        PendingResponse::EventQueueReceive {
            out_ptr: 1,
            payload: Some(EventPayload {
                data3: 9,
                ..payload
            }),
        },
        PendingResponse::CondWakeReacquire {
            mutex_id: 1,
            mutex_kind: CondMutexKind::LwMutex,
        },
        PendingResponse::CondWakeReacquire {
            mutex_id: 1,
            mutex_kind: CondMutexKind::Mutex,
        },
        PendingResponse::EventFlagWake {
            result_ptr: 1,
            observed: 2,
        },
        PendingResponse::EventFlagCancelWake {
            result_ptr: 1,
            observed: 2,
        },
        PendingResponse::LwMutexWake {
            mutex_ptr: 1,
            caller: 2,
        },
    ];
    let partials: Vec<u128> = responses.into_iter().map(partial).collect();
    assert_all_distinct(&partials);
}
