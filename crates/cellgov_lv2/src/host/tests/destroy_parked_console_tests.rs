//! A destroy of a sync object while a thread waits on it answers the
//! `lv2_destroy_parked` probe as the reference console does: the
//! destroy's return code, whether the waiter returned at once, the
//! waiter's return code, and the destroy after the waiter returned,
//! for each of the probe's six cases.
//!
//! A waiter the destroy leaves parked returns when its timeout fires,
//! which here is [`Lv2Host::expire_wait`] on the reason it parked with.

use super::*;
use crate::dispatch::Lv2BlockReason;
use crate::host::test_support::{
    console_payload, extract_write_u32, fake_runtime_with_valid_sync_attr, payload_u32,
    seed_primary_ppu, FakeRuntime, VALID_MUTEX_ATTR_PTR, VALID_SYNC_ATTR_PTR,
};
use crate::ppu_thread::PpuThreadAttrs;
use crate::request::Lv2Request;

const PROBE: &str = "lv2_destroy_parked";

const TIMEOUT_US: u64 = 1_000_000;

const MAIN: UnitId = UnitId::new(0);

const WAITER: UnitId = UnitId::new(1);

/// The probe's cases, in payload order.
#[derive(Clone, Copy, Debug)]
enum Case {
    Semaphore,
    EventFlag,
    Mutex,
    Cond,
    Queue,
    QueueForce,
}

const CASES: [Case; 6] = [
    Case::Semaphore,
    Case::EventFlag,
    Case::Mutex,
    Case::Cond,
    Case::Queue,
    Case::QueueForce,
];

struct World {
    host: Lv2Host,
    rt: FakeRuntime,
}

impl World {
    fn new() -> Self {
        let mut host = Lv2Host::new();
        seed_primary_ppu(&mut host, MAIN);
        host.ppu_threads_mut()
            .create(
                WAITER,
                PpuThreadAttrs {
                    entry: 0x10_1000,
                    arg: 0,
                    stack_base: 0xD001_0000,
                    stack_size: 0x4000,
                    priority: 1000,
                    tls_base: 0,
                },
            )
            .unwrap();
        let rt = fake_runtime_with_valid_sync_attr(0x10000).with_valid_mutex_attr();
        Self { host, rt }
    }

    fn call(&mut self, unit: UnitId, req: Lv2Request) -> Lv2Dispatch {
        self.host.dispatch(req, unit, &self.rt)
    }

    fn created(&mut self, req: Lv2Request) -> u32 {
        match self.call(MAIN, req) {
            Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        }
    }
}

fn code_of(d: &Lv2Dispatch) -> u32 {
    match d {
        Lv2Dispatch::Immediate { code, .. } | Lv2Dispatch::WakeAndReturn { code, .. } => {
            *code as u32
        }
        other => panic!("expected an answer, got {other:?}"),
    }
}

fn destroy(case: Case, id: u32, force: bool) -> Lv2Request {
    match case {
        Case::Semaphore => Lv2Request::SemaphoreDestroy { id },
        Case::EventFlag => Lv2Request::EventFlagDestroy { id },
        Case::Mutex => Lv2Request::MutexDestroy { mutex_id: id },
        Case::Cond => Lv2Request::CondDestroy { id },
        Case::Queue | Case::QueueForce => Lv2Request::EventQueueDestroy {
            queue_id: id,
            mode: u32::from(force),
        },
    }
}

/// CellGov's four words for one case.
fn cellgov_case(case: Case) -> [u32; 4] {
    let mut w = World::new();
    let mut mutex = 0;
    let id = match case {
        Case::Semaphore => w.created(Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            initial: 0,
            max: 1,
        }),
        Case::EventFlag => w.created(Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init: 0,
        }),
        Case::Mutex | Case::Cond => {
            mutex = w.created(Lv2Request::MutexCreate {
                id_ptr: 0x100,
                attr_ptr: VALID_MUTEX_ATTR_PTR,
            });
            if let Case::Mutex = case {
                mutex
            } else {
                w.created(Lv2Request::CondCreate {
                    id_ptr: 0x104,
                    mutex_id: mutex,
                    attr_ptr: 0,
                })
            }
        }
        Case::Queue | Case::QueueForce => w.created(Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 16,
        }),
    };
    if let Case::Mutex = case {
        assert_eq!(
            code_of(&w.call(
                MAIN,
                Lv2Request::MutexLock {
                    mutex_id: id,
                    timeout: 0
                }
            )),
            0
        );
    }
    let wait = match case {
        Case::Semaphore => Lv2Request::SemaphoreWait {
            id,
            timeout: TIMEOUT_US,
        },
        Case::EventFlag => Lv2Request::EventFlagWait {
            id,
            bits: 1,
            mode: 0x02,
            result_ptr: 0x300,
            timeout: TIMEOUT_US,
        },
        Case::Mutex => Lv2Request::MutexLock {
            mutex_id: id,
            timeout: TIMEOUT_US,
        },
        Case::Cond => {
            assert_eq!(
                code_of(&w.call(
                    WAITER,
                    Lv2Request::MutexLock {
                        mutex_id: mutex,
                        timeout: 0
                    }
                )),
                0
            );
            Lv2Request::CondWait {
                id,
                timeout: TIMEOUT_US,
            }
        }
        Case::Queue | Case::QueueForce => Lv2Request::EventQueueReceive {
            queue_id: id,
            out_ptr: 0x300,
            timeout: TIMEOUT_US,
        },
    };
    let reason: Lv2BlockReason = match w.call(WAITER, wait) {
        Lv2Dispatch::Block { reason, .. } => reason,
        other => panic!("{case:?}: the waiter parks, got {other:?}"),
    };

    let parked = w.call(MAIN, destroy(case, id, matches!(case, Case::QueueForce)));
    let woken_now = match &parked {
        Lv2Dispatch::WakeAndReturn {
            woken_unit_ids,
            response_updates,
            ..
        } if woken_unit_ids.contains(&WAITER) => {
            response_updates
                .iter()
                .find_map(|(u, r)| match (u == &WAITER, r) {
                    (true, PendingResponse::ReturnCode { code }) => Some(*code as u32),
                    _ => None,
                })
        }
        _ => None,
    };
    let waiter_rc = match woken_now {
        Some(code) => code,
        None => {
            let expired = w.host.expire_wait(reason, WAITER, GuestTicks::ZERO);
            match expired.response_updates.as_slice() {
                [(unit, PendingResponse::ReturnCode { code })] if *unit == WAITER => *code as u32,
                other => panic!("{case:?}: one ReturnCode for the waiter, got {other:?}"),
            }
        }
    };
    match case {
        Case::Mutex => {
            assert_eq!(
                code_of(&w.call(MAIN, Lv2Request::MutexUnlock { mutex_id: id })),
                0
            );
        }
        Case::Cond => {
            assert_eq!(
                code_of(&w.call(WAITER, Lv2Request::MutexUnlock { mutex_id: mutex })),
                0
            );
        }
        _ => {}
    }
    let after = w.call(MAIN, destroy(case, id, false));
    [
        code_of(&parked),
        u32::from(woken_now.is_some()),
        waiter_rc,
        code_of(&after),
    ]
}

#[test]
fn destroying_an_object_with_a_parked_waiter_answers_as_the_console_does() {
    let payload = console_payload(PROBE);
    assert_eq!(
        (payload_u32(&payload, 0), payload_u32(&payload, 4)),
        (0, 0),
        "the probe ran every step"
    );
    for (i, case) in CASES.into_iter().enumerate() {
        let at = 8 + 16 * i;
        let console = [0, 4, 8, 12].map(|off| payload_u32(&payload, at + off));
        assert_eq!(
            cellgov_case(case),
            console,
            "{case:?}: [destroy while parked, returned at once, waiter rc, destroy after]"
        );
    }
}
