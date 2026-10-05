//! The lwmutex, mutex, cond and rwlock answers of the
//! `lv2_mutex_cond_rwlock_rows` probe, held to its console capture.
//!
//! Every return code and wake ordering matches the console. The object
//! ids do not: the console answers class-tagged ids (an lwmutex id
//! carries 0x95 in its top byte, an rwlock id 0x88), and CellGov
//! answers small integers.
//! `object_ids_differ_from_the_consoles_class_tagged_form` names that
//! divergence.

use super::*;
use crate::dispatch::PendingResponse;
use crate::host::test_support::{
    console_payload, extract_write_u32, payload_u32, seed_primary_ppu, FakeRuntime, COND_ATTR_PTR,
    VALID_MUTEX_ATTR_PTR,
};
use crate::ppu_thread::PpuThreadAttrs;
use crate::request::Lv2Request;

const PROBE: &str = "lv2_mutex_cond_rwlock_rows";

const TIMEOUT_US: u64 = 1_000_000;

const MAIN: UnitId = UnitId::new(0);

fn word(payload: &[u8], i: usize) -> u32 {
    payload_u32(payload, 4 * i)
}

struct World {
    host: Lv2Host,
    rt: FakeRuntime,
    /// The return code each parked unit gets when it wakes. A signal
    /// sets it; the mutex unlock that later wakes the unit carries none.
    codes: std::collections::BTreeMap<UnitId, u32>,
}

impl World {
    fn new(helpers: u32) -> Self {
        let mut host = Lv2Host::new();
        seed_primary_ppu(&mut host, MAIN);
        for i in 1..=helpers {
            host.ppu_threads_mut()
                .create(
                    UnitId::new(u64::from(i)),
                    PpuThreadAttrs {
                        entry: 0x10_1000,
                        arg: 0,
                        stack_base: 0xD001_0000 + 0x1_0000 * i,
                        stack_size: 0x4000,
                        priority: 1000,
                        tls_base: 0,
                    },
                )
                .unwrap();
        }
        Self {
            host,
            rt: FakeRuntime::new(0x10000).with_valid_mutex_attr(),
            codes: std::collections::BTreeMap::new(),
        }
    }

    fn call(&mut self, unit: UnitId, req: Lv2Request) -> Lv2Dispatch {
        let d = self.host.dispatch(req, unit, &self.rt);
        if let Lv2Dispatch::WakeAndReturn {
            response_updates, ..
        } = &d
        {
            for (u, r) in response_updates {
                if let PendingResponse::ReturnCode { code } = r {
                    self.codes.insert(*u, *code as u32);
                }
            }
        }
        d
    }

    /// The units a dispatch woke, each with the return code that the
    /// signal which released it recorded.
    fn woken(&mut self, d: &Lv2Dispatch) -> Vec<(UnitId, u32)> {
        let Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } = d else {
            return Vec::new();
        };
        woken_unit_ids
            .iter()
            .map(|u| {
                let code = self
                    .codes
                    .remove(u)
                    .unwrap_or_else(|| panic!("{u:?} woke with no recorded return code"));
                (*u, code)
            })
            .collect()
    }

    fn rc(&mut self, unit: UnitId, req: Lv2Request) -> u32 {
        match self.call(unit, req) {
            Lv2Dispatch::Immediate { code, .. } | Lv2Dispatch::WakeAndReturn { code, .. } => {
                code as u32
            }
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    fn created(&mut self, req: Lv2Request) -> (u32, u32) {
        match self.call(MAIN, req) {
            Lv2Dispatch::Immediate { code: 0, effects } => (0, extract_write_u32(&effects[0])),
            Lv2Dispatch::Immediate { code, .. } => (code as u32, 0),
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    fn mutex(&mut self) -> (u32, u32) {
        self.created(Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_MUTEX_ATTR_PTR,
        })
    }

    fn cond(&mut self, mutex_id: u32) -> (u32, u32) {
        self.created(Lv2Request::CondCreate {
            id_ptr: 0x104,
            mutex_id,
            attr_ptr: COND_ATTR_PTR,
        })
    }

    /// `unit` locks the mutex and parks on the cond.
    fn park_on_cond(&mut self, unit: UnitId, mutex_id: u32, cond_id: u32) {
        let lock = Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        };
        assert_eq!(self.rc(unit, lock), 0);
        let wait = Lv2Request::CondWait {
            id: cond_id,
            timeout: TIMEOUT_US,
        };
        assert!(matches!(self.call(unit, wait), Lv2Dispatch::Block { .. }));
    }
}

#[test]
fn lwmutex_rwlock_and_mutex_answers_match_the_console() {
    let p = console_payload(PROBE);
    assert_eq!(word(&p, 39), 0, "the probe ran every step");
    let mut w = World::new(1);
    let holder = UnitId::new(1);

    // A: three lwmutex creates and destroys (ids compared elsewhere).
    let mut ids = Vec::new();
    for i in 0..3usize {
        let (rc, id) = w.created(Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200 + 0x20 * i as u32,
        });
        assert_eq!(rc, word(&p, i), "lwmutex create {i}");
        ids.push(id);
    }
    for (i, id) in ids.into_iter().enumerate() {
        let rc = w.rc(MAIN, Lv2Request::LwMutexDestroy { id });
        assert_eq!(rc, word(&p, 6 + i), "lwmutex destroy {i}");
    }

    // B: rwlock.
    assert_eq!(
        w.rc(MAIN, Lv2Request::RwlockDestroy { id: 0xDEAD_BEEF }),
        word(&p, 9),
        "stale rwlock destroy"
    );
    let (rc, rw) = w.created(Lv2Request::RwlockCreate {
        id_ptr: 0x100,
        attr_ptr: 0x300,
    });
    assert_eq!(rc, word(&p, 10), "rwlock create");
    assert_eq!(
        w.rc(MAIN, Lv2Request::RwlockDestroy { id: rw }),
        word(&p, 12)
    );
    assert_eq!(
        w.rc(MAIN, Lv2Request::RwlockDestroy { id: rw }),
        word(&p, 13),
        "rwlock destroyed twice"
    );

    // C: trylock while another thread holds the mutex.
    let (rc, m) = w.mutex();
    assert_eq!(rc, word(&p, 14));
    let lock = Lv2Request::MutexLock {
        mutex_id: m,
        timeout: 0,
    };
    assert_eq!(w.rc(holder, lock), 0);
    assert_eq!(
        w.rc(MAIN, Lv2Request::MutexTryLock { mutex_id: m }),
        word(&p, 15),
        "contended trylock"
    );
    assert_eq!(w.rc(holder, Lv2Request::MutexUnlock { mutex_id: m }), 0);
    assert_eq!(
        w.rc(MAIN, Lv2Request::MutexTryLock { mutex_id: m }),
        word(&p, 16)
    );
    assert_eq!(
        w.rc(MAIN, Lv2Request::MutexUnlock { mutex_id: m }),
        word(&p, 17)
    );
    assert_eq!(
        w.rc(MAIN, Lv2Request::MutexDestroy { mutex_id: m }),
        word(&p, 18)
    );
    assert_eq!(
        w.rc(MAIN, Lv2Request::MutexDestroy { mutex_id: m }),
        word(&p, 19),
        "mutex destroyed twice"
    );
}

#[test]
fn cond_signals_wake_waiters_in_the_consoles_order() {
    let p = console_payload(PROBE);
    let mut w = World::new(2);
    let (w1, w2) = (UnitId::new(1), UnitId::new(2));

    // D: a signal while main holds the mutex.
    let (rc, m) = w.mutex();
    assert_eq!(rc, word(&p, 20));
    let (rc, c) = w.cond(m);
    assert_eq!(rc, word(&p, 21));
    w.park_on_cond(w1, m, c);
    let lock = Lv2Request::MutexLock {
        mutex_id: m,
        timeout: 0,
    };
    assert_eq!(w.rc(MAIN, lock), word(&p, 22));
    let signal = w.call(MAIN, Lv2Request::CondSignal { id: c });
    let returned_early = !w.woken(&signal).is_empty();
    assert_eq!(
        (code_or_zero(&signal), u32::from(returned_early)),
        (word(&p, 23), word(&p, 24)),
        "signal while the mutex is held: [rc, waiter returned]"
    );
    let unlock = w.call(MAIN, Lv2Request::MutexUnlock { mutex_id: m });
    let after = w.woken(&unlock);
    assert_eq!(code_or_zero(&unlock), word(&p, 25));
    let waiter = after.iter().find(|(u, _)| *u == w1);
    assert_eq!(u32::from(waiter.is_some()), word(&p, 26));
    assert_eq!(
        waiter.map_or(u32::MAX, |(_, code)| *code),
        word(&p, 27),
        "the waiter's cond_wait answer"
    );
    assert_eq!(w.rc(w1, Lv2Request::MutexUnlock { mutex_id: m }), 0);

    // E: signal_all with two waiters and the mutex free.
    w.park_on_cond(w1, m, c);
    w.park_on_cond(w2, m, c);
    let all = w.call(MAIN, Lv2Request::CondSignalAll { id: c });
    assert_eq!(code_or_zero(&all), word(&p, 28));
    let mut returned = w.woken(&all);
    // The first waiter holds the mutex; its unlock hands it on.
    for (unit, _) in returned.clone() {
        let unlock = w.call(unit, Lv2Request::MutexUnlock { mutex_id: m });
        let next = w.woken(&unlock);
        for (n, _) in &next {
            assert_eq!(w.rc(*n, Lv2Request::MutexUnlock { mutex_id: m }), 0);
        }
        returned.extend(next);
    }
    assert_eq!(returned.len() as u32, word(&p, 29), "waiters returned");
    for (i, (_, code)) in returned.iter().enumerate() {
        assert_eq!(*code, word(&p, 30 + i), "signal_all waiter {i}");
    }

    // F: signal_to the parked waiter, then to a bogus thread id.
    w.park_on_cond(w1, m, c);
    let target = w.host.ppu_threads().thread_id_for_unit(w1).unwrap().raw() as u32;
    let to = w.call(
        MAIN,
        Lv2Request::CondSignalTo {
            id: c,
            target_thread: target,
        },
    );
    assert_eq!(code_or_zero(&to), word(&p, 32));
    assert_eq!(
        w.rc(
            MAIN,
            Lv2Request::CondSignalTo {
                id: c,
                target_thread: 0xDEAD_BEEF
            }
        ),
        word(&p, 33),
        "signal_to a bogus thread id"
    );
    assert_eq!(w.woken(&to), vec![(w1, word(&p, 34))]);
    assert_eq!(w.rc(w1, Lv2Request::MutexUnlock { mutex_id: m }), 0);

    // G: no waiter, a stale cond, cleanup.
    assert_eq!(w.rc(MAIN, Lv2Request::CondSignal { id: c }), word(&p, 35));
    assert_eq!(
        w.rc(MAIN, Lv2Request::CondSignal { id: 0xDEAD_BEEF }),
        word(&p, 36)
    );
    assert_eq!(w.rc(MAIN, Lv2Request::CondDestroy { id: c }), word(&p, 37));
    assert_eq!(
        w.rc(MAIN, Lv2Request::MutexDestroy { mutex_id: m }),
        word(&p, 38)
    );
}

fn code_or_zero(d: &Lv2Dispatch) -> u32 {
    match d {
        Lv2Dispatch::Immediate { code, .. } | Lv2Dispatch::WakeAndReturn { code, .. } => {
            *code as u32
        }
        other => panic!("expected an answer, got {other:?}"),
    }
}

#[test]
fn object_ids_differ_from_the_consoles_class_tagged_form() {
    let p = console_payload(PROBE);
    // The console: lwmutex ids carry class byte 0x95 and step by 0x100;
    // the rwlock id carries 0x88.
    let lw: Vec<u32> = (3..=5).map(|i| word(&p, i)).collect();
    assert!(lw.iter().all(|id| id >> 24 == 0x95), "{lw:x?}");
    assert_eq!(lw[1] - lw[0], 0x100);
    assert_eq!(lw[2] - lw[1], 0x100);
    assert_eq!(word(&p, 11) >> 24, 0x88);

    // CellGov: small integers, so a guest that reads the class byte of
    // an id sees 0. This assertion fails once ids take the console's
    // form, which is when this test and the one above it change.
    let mut w = World::new(0);
    let (_, first) = w.created(Lv2Request::LwMutexCreate {
        id_ptr: 0x100,
        attr_ptr: 0x200,
    });
    assert_eq!(first >> 24, 0);
}

const COND_PROBE: &str = "lv2_cond_create_gates";

#[test]
fn cond_create_checks_its_attribute_block_first_as_the_console_does() {
    let p = console_payload(COND_PROBE);
    let mut w = World::new(0);
    let (rc, m) = w.mutex();
    assert_eq!(rc, word(&p, 0));
    let create = |w: &mut World, id_ptr: u32, mutex_id: u32, attr_ptr: u32| {
        w.call(
            MAIN,
            Lv2Request::CondCreate {
                id_ptr,
                mutex_id,
                attr_ptr,
            },
        )
    };
    let null_attr = create(&mut w, 0x104, m, 0);
    let wrote_id = match &null_attr {
        Lv2Dispatch::Immediate { effects, .. } => u32::from(!effects.is_empty()),
        other => panic!("expected Immediate, got {other:?}"),
    };
    assert_eq!(
        [code_or_zero(&null_attr), wrote_id],
        [word(&p, 1), word(&p, 2)],
        "null attribute block: [rc, id written]"
    );
    assert_eq!(word(&p, 3), 0xFFFF_FFFF, "the probe destroyed nothing");
    let null_id = create(&mut w, 0, m, COND_ATTR_PTR);
    assert_eq!(code_or_zero(&null_id), word(&p, 4), "null id pointer");
    let stale = create(&mut w, 0x104, 0xDEAD_BEEF, COND_ATTR_PTR);
    assert_eq!(code_or_zero(&stale), word(&p, 5), "stale mutex");
    let both = create(&mut w, 0x104, 0xDEAD_BEEF, 0);
    assert_eq!(
        code_or_zero(&both),
        word(&p, 6),
        "stale mutex and null attribute block"
    );
    assert_eq!(
        w.rc(MAIN, Lv2Request::MutexDestroy { mutex_id: m }),
        word(&p, 7)
    );
}
