//! Wait-family timeout extraction.

use super::*;

#[test]
fn every_wait_family_request_exposes_its_timeout() {
    let cases: Vec<Lv2Request> = vec![
        Lv2Request::MutexLock {
            mutex_id: 1,
            timeout: 5,
        },
        Lv2Request::SemaphoreWait { id: 1, timeout: 5 },
        Lv2Request::EventQueueReceive {
            queue_id: 1,
            out_ptr: 0x100,
            timeout: 5,
        },
        Lv2Request::EventFlagWait {
            id: 1,
            bits: 0b1,
            mode: 0x01,
            result_ptr: 0x100,
            timeout: 5,
        },
        Lv2Request::LwMutexLock {
            id: 1,
            mutex_ptr: 0x100,
            timeout: 5,
        },
        Lv2Request::CondWait { id: 1, timeout: 5 },
    ];
    for req in cases {
        assert_eq!(req.wait_timeout_usec(), Some(5), "for {req:?}");
    }
}

#[test]
fn zero_timeout_is_reported_as_zero_not_none() {
    // 0 means wait-forever; the caller (the runtime's deadline
    // registration) is the one that maps 0 to "no deadline". The
    // extractor must not conflate it with a non-wait request.
    let req = Lv2Request::MutexLock {
        mutex_id: 1,
        timeout: 0,
    };
    assert_eq!(req.wait_timeout_usec(), Some(0));
}

#[test]
fn non_wait_requests_have_no_timeout() {
    let cases: Vec<Lv2Request> = vec![
        Lv2Request::MutexTryLock { mutex_id: 1 },
        Lv2Request::SemaphoreTryWait { id: 1 },
        Lv2Request::EventFlagTryWait {
            id: 1,
            bits: 0b1,
            mode: 0x01,
            result_ptr: 0x100,
        },
        Lv2Request::PpuThreadJoin {
            target: 1,
            status_out_ptr: 0x100,
        },
        Lv2Request::SpuThreadGroupJoin {
            group_id: 1,
            cause_ptr: 0x100,
            status_ptr: 0x104,
        },
        Lv2Request::PpuThreadYield,
    ];
    for req in cases {
        assert_eq!(req.wait_timeout_usec(), None, "for {req:?}");
    }
}
