//! Sync-state lanes of the dispatch types that other tables store: a
//! block reason and a pending response.
//!
//! Each variant takes a tag lane (its tag + 1, never 0) and one lane per
//! payload field, so two variants whose payloads share values still
//! differ.

use cellgov_mem::lanes::{LaneValue, ObjectLanes};

use crate::dispatch::{Lv2BlockReason, PendingResponse};

impl Lv2BlockReason {
    /// Add the reason's lanes to `lanes`: the tag at `field`, the payload
    /// at the fields after it. Returns the next free field.
    pub fn push_lanes(&self, lanes: &mut ObjectLanes, field: u8) -> u8 {
        let (tag, payload): (u64, [u64; 3]) = match *self {
            Lv2BlockReason::ThreadGroupJoin { group_id } => (0, [u64::from(group_id), 0, 0]),
            Lv2BlockReason::PpuThreadJoin { target } => (1, [target, 0, 0]),
            Lv2BlockReason::LwMutex { id } => (2, [u64::from(id), 0, 0]),
            Lv2BlockReason::Mutex { id } => (3, [u64::from(id), 0, 0]),
            Lv2BlockReason::Semaphore { id } => (4, [u64::from(id), 0, 0]),
            Lv2BlockReason::EventQueue { id } => (5, [u64::from(id), 0, 0]),
            Lv2BlockReason::EventFlag { id } => (6, [u64::from(id), 0, 0]),
            Lv2BlockReason::Cond {
                id,
                mutex_id,
                mutex_kind,
            } => (
                7,
                [u64::from(id), u64::from(mutex_id), mutex_kind as u64 + 1],
            ),
            Lv2BlockReason::Uart => (8, [0, 0, 0]),
            Lv2BlockReason::UsbdEvent { handle } => (9, [u64::from(handle), 0, 0]),
        };
        lanes.lane(field, 0, tag + 1);
        for (i, value) in payload.into_iter().enumerate() {
            lanes.lane(field + 1 + i as u8, 0, value);
        }
        field + 4
    }
}

/// Field 1 is the variant tag; fields 2 to 7 are the payload.
impl LaneValue for PendingResponse {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        let payload: [u64; 6] = match *self {
            PendingResponse::ReturnCode { code } => [code, 0, 0, 0, 0, 0],
            PendingResponse::ThreadGroupJoin {
                group_id,
                code,
                cause_ptr,
                status_ptr,
                cause,
                status,
            } => [
                u64::from(group_id),
                code,
                u64::from(cause_ptr),
                u64::from(status_ptr),
                u64::from(cause),
                u64::from(status),
            ],
            PendingResponse::PpuThreadJoin {
                target,
                status_out_ptr,
            } => [target, u64::from(status_out_ptr), 0, 0, 0, 0],
            PendingResponse::EventQueueReceive { out_ptr, payload } => match payload {
                None => [u64::from(out_ptr), 0, 0, 0, 0, 0],
                Some(p) => [u64::from(out_ptr), 1, p.source, p.data1, p.data2, p.data3],
            },
            PendingResponse::CondWakeReacquire {
                mutex_id,
                mutex_kind,
            } => [u64::from(mutex_id), mutex_kind as u64 + 1, 0, 0, 0, 0],
            PendingResponse::EventFlagWake {
                result_ptr,
                observed,
            } => [u64::from(result_ptr), observed, 0, 0, 0, 0],
            PendingResponse::LwMutexWake { mutex_ptr, caller } => {
                [u64::from(mutex_ptr), u64::from(caller), 0, 0, 0, 0]
            }
            PendingResponse::EventFlagCancelWake {
                result_ptr,
                observed,
            } => [u64::from(result_ptr), observed, 0, 0, 0, 0],
        };
        lanes.lane(1, 0, u64::from(self.variant_tag()) + 1);
        for (i, value) in payload.into_iter().enumerate() {
            lanes.lane(2 + i as u8, 0, value);
        }
    }
}

#[cfg(test)]
#[path = "tests/dispatch_lanes_tests.rs"]
mod tests;
