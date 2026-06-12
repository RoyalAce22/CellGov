//! Validation of pending syscall-response updates against the woken-unit set.

use super::*;
use crate::syscall_table::SyscallResponseTable;
use cellgov_lv2::EventPayload;

#[test]
#[should_panic(expected = "is not in woken_unit_ids")]
fn check_response_updates_rejects_update_for_non_woken_unit() {
    let table = SyscallResponseTable::new();
    let waiter = UnitId::new(42);
    let updates = vec![(waiter, PendingResponse::ReturnCode { code: 0 })];
    check_response_updates("test", &table, &[], &updates);
}

#[test]
#[should_panic(expected = "variant mismatch")]
fn check_response_updates_rejects_variant_mismatch() {
    let mut table = SyscallResponseTable::new();
    let waiter = UnitId::new(7);
    let _ = table.insert(
        waiter,
        PendingResponse::EventQueueReceive {
            out_ptr: 0x1000,
            payload: None,
        },
    );
    let updates = vec![(
        waiter,
        PendingResponse::EventFlagWake {
            result_ptr: 0x1000,
            observed: 0,
        },
    )];
    check_response_updates("test", &table, &[waiter], &updates);
}

#[test]
fn check_response_updates_allows_return_code_to_replace_any_variant() {
    let mut table = SyscallResponseTable::new();
    let waiter = UnitId::new(7);
    let _ = table.insert(
        waiter,
        PendingResponse::EventFlagWake {
            result_ptr: 0x1000,
            observed: 0,
        },
    );
    let updates = vec![(waiter, PendingResponse::ReturnCode { code: 0x80010013 })];
    check_response_updates("test", &table, &[waiter], &updates);
}

#[test]
fn check_response_updates_accepts_same_variant_fill() {
    let mut table = SyscallResponseTable::new();
    let waiter = UnitId::new(7);
    let _ = table.insert(
        waiter,
        PendingResponse::EventQueueReceive {
            out_ptr: 0x1000,
            payload: None,
        },
    );
    let updates = vec![(
        waiter,
        PendingResponse::EventQueueReceive {
            out_ptr: 0x1000,
            payload: Some(EventPayload {
                source: 0x11,
                data1: 0x22,
                data2: 0x33,
                data3: 0x44,
            }),
        },
    )];
    check_response_updates("test", &table, &[waiter], &updates);
}
