use super::*;
use crate::dispatch::ExpiredWait;
use crate::host::test_support::{extract_write_u32, seed_primary_ppu, FakeRuntime};
use crate::ppu_thread::PpuThreadAttrs;
use crate::request::Lv2Request;
use cellgov_ps3_abi::lv2::sync::lwcond_signal_mode::{FORCED, NOT_OWNED, OWNED};

const A: UnitId = UnitId::new(0);
const B: UnitId = UnitId::new(1);
const C: UnitId = UnitId::new(2);
const ANY: u32 = LWCOND_SIGNAL_ANY_THREAD;

/// A host with three PPU threads (units A, B, C), one lwmutex and one
/// lwcond bound to it. The lwmutex's kernel entry starts unsignalled,
/// as when user space holds it.
struct Fixture {
    host: Lv2Host,
    rt: FakeRuntime,
    lwmutex: u32,
    lwcond: u32,
}

impl Fixture {
    fn new() -> Self {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        seed_primary_ppu(&mut host, A);
        for unit in [B, C] {
            host.ppu_threads_mut()
                .create(
                    unit,
                    PpuThreadAttrs {
                        entry: 0,
                        arg: 0,
                        stack_base: 0,
                        stack_size: 0,
                        priority: 0,
                        tls_base: 0,
                    },
                )
                .expect("thread creates");
        }
        let mut f = Self {
            host,
            rt,
            lwmutex: 0,
            lwcond: 0,
        };
        f.lwmutex = f.written(Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        });
        f.lwcond = f.written(Lv2Request::LwCondCreate {
            id_ptr: 0x104,
            lwmutex_id: f.lwmutex,
            control_ptr: 0x300,
            name: 0,
        });
        f
    }

    fn go(&mut self, request: Lv2Request, unit: UnitId) -> Lv2Dispatch {
        self.host.dispatch(request, unit, &self.rt)
    }

    fn code(&mut self, request: Lv2Request, unit: UnitId) -> u64 {
        match self.go(request, unit) {
            Lv2Dispatch::Immediate { code, .. } => code,
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    fn written(&mut self, request: Lv2Request) -> u32 {
        match self.go(request, A) {
            Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate(0) with a write, got {other:?}"),
        }
    }

    fn tid(&self, unit: UnitId) -> PpuThreadId {
        self.host.ppu_threads().thread_id_for_unit(unit).unwrap()
    }

    fn wait(&mut self, unit: UnitId) -> Lv2Dispatch {
        let request = Lv2Request::LwCondQueueWait {
            id: self.lwcond,
            lwmutex_id: self.lwmutex,
            timeout: 0,
        };
        self.go(request, unit)
    }

    fn lock(&mut self, unit: UnitId) -> Lv2Dispatch {
        let request = Lv2Request::LwMutexLock {
            id: self.lwmutex,
            mutex_ptr: 0,
            timeout: 0,
        };
        self.go(request, unit)
    }

    fn unlock(&mut self, unit: UnitId) -> Lv2Dispatch {
        let request = Lv2Request::LwMutexUnlock { id: self.lwmutex };
        self.go(request, unit)
    }

    fn signal(&mut self, unit: UnitId, thread: u32, mode: u32) -> Lv2Dispatch {
        let request = Lv2Request::LwCondSignal {
            id: self.lwcond,
            lwmutex_id: self.lwmutex,
            ppu_thread_id: thread,
            mode,
        };
        self.go(request, unit)
    }

    fn signal_all(&mut self, unit: UnitId, mode: u32) -> Lv2Dispatch {
        let request = Lv2Request::LwCondSignalAll {
            id: self.lwcond,
            lwmutex_id: self.lwmutex,
            mode,
        };
        self.go(request, unit)
    }

    /// `unit` waits on the lwcond, then A takes the lwmutex the wait
    /// released.
    fn park_then_own(&mut self, unit: UnitId) {
        assert!(matches!(self.wait(unit), Lv2Dispatch::Block { .. }));
        assert!(matches!(
            self.lock(A),
            Lv2Dispatch::Immediate { code: 0, .. }
        ));
    }

    fn lwcond_waiters(&self) -> Vec<PpuThreadId> {
        self.host
            .state
            .lwconds
            .lookup(self.lwcond)
            .unwrap()
            .waiters()
            .iter()
            .collect()
    }

    fn lwmutex_waiters(&self) -> Vec<PpuThreadId> {
        let entry = self.host.state.lwmutexes.lookup(self.lwmutex).unwrap();
        entry.waiters().iter().collect()
    }
}

fn woken(d: &Lv2Dispatch) -> (u64, Vec<UnitId>, Vec<(UnitId, PendingResponse)>) {
    match d {
        Lv2Dispatch::WakeAndReturn {
            code,
            woken_unit_ids,
            response_updates,
            ..
        } => (*code, woken_unit_ids.clone(), response_updates.clone()),
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
}

fn ret(code: impl Into<u64>) -> PendingResponse {
    PendingResponse::ReturnCode { code: code.into() }
}

#[test]
fn a_create_binds_a_live_lwmutex_and_a_destroy_removes_it() {
    let mut f = Fixture::new();
    let unknown = Lv2Request::LwCondCreate {
        id_ptr: 0x108,
        lwmutex_id: 0x7777,
        control_ptr: 0x300,
        name: 0,
    };
    assert_eq!(f.code(unknown, A), u64::from(errno::CELL_ESRCH));
    assert_eq!(
        f.host
            .state
            .lwconds
            .lookup(f.lwcond)
            .map(|e| e.lwmutex_id()),
        Some(f.lwmutex)
    );
    let count = |f: &Fixture| {
        f.host
            .state
            .process_counts
            .count_for_class(cellgov_ps3_abi::lv2::process::SYS_LWCOND_OBJECT, &f.host)
    };
    assert_eq!(count(&f), 1);
    let destroy = Lv2Request::LwCondDestroy { id: f.lwcond };
    assert_eq!(f.code(destroy, A), 0);
    assert_eq!(count(&f), 0);
    assert_eq!(f.code(destroy, A), u64::from(errno::CELL_ESRCH));
}

#[test]
fn a_wait_parks_and_releases_the_lwmutex_to_the_next_locker() {
    let mut f = Fixture::new();
    let tid_b = f.tid(B);
    match f.wait(B) {
        Lv2Dispatch::Block {
            reason, pending, ..
        } => {
            assert_eq!(
                reason,
                Lv2BlockReason::LwCond {
                    id: f.lwcond,
                    lwmutex_id: f.lwmutex
                }
            );
            assert_eq!(
                pending,
                PendingResponse::LwMutexWake {
                    mutex_ptr: 0,
                    caller: tid_b.raw() as u32
                }
            );
        }
        other => panic!("expected Block, got {other:?}"),
    }
    assert_eq!(f.lwcond_waiters(), vec![tid_b]);
    assert!(
        matches!(f.lock(C), Lv2Dispatch::Immediate { code: 0, .. }),
        "the wait left the lwmutex signalled"
    );
    let destroy = Lv2Request::LwCondDestroy { id: f.lwcond };
    assert_eq!(f.code(destroy, A), u64::from(errno::CELL_EBUSY));
}

#[test]
fn a_wait_hands_the_lwmutex_to_its_sleeper() {
    let mut f = Fixture::new();
    assert!(matches!(f.lock(C), Lv2Dispatch::Block { .. }));
    match f.wait(B) {
        Lv2Dispatch::BlockAndWake { woken_unit_ids, .. } => assert_eq!(woken_unit_ids, vec![C]),
        other => panic!("expected BlockAndWake, got {other:?}"),
    }
}

#[test]
fn an_owner_signal_moves_the_waiter_onto_the_lwmutex_until_the_unlock() {
    let mut f = Fixture::new();
    f.park_then_own(B);
    let tid_b = f.tid(B);
    assert!(matches!(
        f.signal(A, ANY, OWNED),
        Lv2Dispatch::Immediate { code: 0, .. }
    ));
    assert!(f.lwcond_waiters().is_empty());
    assert_eq!(f.lwmutex_waiters(), vec![tid_b]);
    let (code, units, updates) = woken(&f.unlock(A));
    assert_eq!((code, units, updates), (0, vec![B], vec![]));
}

#[test]
fn a_signal_that_could_not_lock_wakes_the_waiter_with_ebusy() {
    let mut f = Fixture::new();
    assert!(matches!(f.wait(B), Lv2Dispatch::Block { .. }));
    let (code, units, updates) = woken(&f.signal(C, ANY, NOT_OWNED));
    assert_eq!(code, 0);
    assert_eq!(units, vec![B]);
    assert_eq!(updates, vec![(B, ret(errno::CELL_EBUSY))]);
}

#[test]
fn a_forced_signal_gives_the_waiter_the_lwmutex_when_none_sleeps() {
    let mut f = Fixture::new();
    f.park_then_own(B);
    let (code, units, updates) = woken(&f.signal(A, ANY, FORCED));
    assert_eq!((code, units, updates), (0, vec![B], vec![]));
    assert!(f.lwmutex_waiters().is_empty());
}

#[test]
fn a_forced_signal_gives_the_lwmutex_to_its_oldest_sleeper_first() {
    let mut f = Fixture::new();
    f.park_then_own(B);
    assert!(matches!(f.lock(C), Lv2Dispatch::Block { .. }));
    let tid_b = f.tid(B);
    let (code, units, updates) = woken(&f.signal(A, ANY, FORCED));
    assert_eq!((code, units, updates), (0, vec![C], vec![]));
    assert_eq!(f.lwmutex_waiters(), vec![tid_b]);
}

#[test]
fn a_signal_names_its_waiter_and_answers_when_none_waits() {
    let mut f = Fixture::new();
    let tid_c = f.tid(C).raw() as u32;
    let none = |f: &mut Fixture, thread, mode| match f.signal(A, thread, mode) {
        Lv2Dispatch::Immediate { code, .. } => code,
        other => panic!("expected Immediate, got {other:?}"),
    };
    assert_eq!(none(&mut f, ANY, NOT_OWNED), 0);
    assert_eq!(none(&mut f, ANY, FORCED), u64::from(errno::CELL_ENOENT));
    assert_eq!(none(&mut f, ANY, OWNED), u64::from(errno::CELL_EPERM));
    assert_eq!(none(&mut f, tid_c, NOT_OWNED), u64::from(errno::CELL_EPERM));
    assert_eq!(none(&mut f, 0x7777, OWNED), u64::from(errno::CELL_ESRCH));
    assert_eq!(none(&mut f, ANY, 4), u64::from(errno::CELL_EINVAL));

    assert!(matches!(f.wait(B), Lv2Dispatch::Block { .. }));
    assert!(matches!(f.wait(C), Lv2Dispatch::Block { .. }));
    let (_, units, _) = woken(&f.signal(A, tid_c, NOT_OWNED));
    assert_eq!(units, vec![C], "the named thread wakes, not the head");
    assert_eq!(f.lwcond_waiters(), vec![f.tid(B)]);
}

#[test]
fn a_signal_needing_a_destroyed_lwmutex_is_esrch_and_leaves_the_waiter() {
    let mut f = Fixture::new();
    assert!(matches!(f.wait(B), Lv2Dispatch::Block { .. }));
    let destroy = Lv2Request::LwMutexDestroy { id: f.lwmutex };
    assert_eq!(f.code(destroy, A), 0);
    for mode in [OWNED, FORCED] {
        assert!(matches!(
            f.signal(A, ANY, mode),
            Lv2Dispatch::Immediate { code, .. } if code == u64::from(errno::CELL_ESRCH)
        ));
    }
    assert_eq!(f.lwcond_waiters(), vec![f.tid(B)]);
    let (_, units, updates) = woken(&f.signal(A, ANY, NOT_OWNED));
    assert_eq!(units, vec![B]);
    assert_eq!(updates, vec![(B, ret(errno::CELL_EBUSY))]);
}

#[test]
fn a_parked_waiter_reaches_the_state_hash() {
    let mut released = Fixture::new();
    assert!(matches!(
        released.unlock(A),
        Lv2Dispatch::Immediate { code: 0, .. }
    ));
    let mut parked = Fixture::new();
    assert!(matches!(parked.wait(B), Lv2Dispatch::Block { .. }));
    assert_eq!(
        released.host.state.lwmutexes.lookup(released.lwmutex),
        parked.host.state.lwmutexes.lookup(parked.lwmutex),
        "both leave the lwmutex released"
    );
    assert_ne!(released.host.sync_partial(), parked.host.sync_partial());
}

#[test]
fn a_signal_all_moves_every_waiter_or_wakes_each_with_ebusy() {
    let mut f = Fixture::new();
    assert!(matches!(f.wait(B), Lv2Dispatch::Block { .. }));
    assert!(matches!(f.wait(C), Lv2Dispatch::Block { .. }));
    assert!(matches!(f.lock(A), Lv2Dispatch::Immediate { code: 0, .. }));
    assert!(matches!(
        f.signal_all(A, OWNED),
        Lv2Dispatch::Immediate { code: 2, .. }
    ));
    let (tid_b, tid_c) = (f.tid(B), f.tid(C));
    assert_eq!(f.lwmutex_waiters(), vec![tid_b, tid_c]);

    let mut f = Fixture::new();
    assert!(matches!(f.wait(B), Lv2Dispatch::Block { .. }));
    assert!(matches!(f.wait(C), Lv2Dispatch::Block { .. }));
    let (code, units, updates) = woken(&f.signal_all(A, NOT_OWNED));
    assert_eq!(code, 0);
    assert_eq!(units, vec![B, C]);
    assert_eq!(
        updates,
        vec![(B, ret(errno::CELL_EBUSY)), (C, ret(errno::CELL_EBUSY))]
    );
    assert!(matches!(
        f.signal_all(A, FORCED),
        Lv2Dispatch::Immediate { code, .. } if code == u64::from(errno::CELL_EINVAL)
    ));
}

#[test]
fn a_timeout_takes_the_waiter_off_either_queue() {
    let mut f = Fixture::new();
    let reason = Lv2BlockReason::LwCond {
        id: f.lwcond,
        lwmutex_id: f.lwmutex,
    };
    let timed_out = |out: ExpiredWait| {
        assert_eq!(out.woken_unit_ids, vec![B]);
        assert_eq!(out.response_updates, vec![(B, ret(errno::CELL_ETIMEDOUT))]);
    };
    assert!(matches!(f.wait(B), Lv2Dispatch::Block { .. }));
    timed_out(f.host.expire_wait(reason, B, GuestTicks::ZERO));
    assert!(f.lwcond_waiters().is_empty());

    f.park_then_own(B);
    assert!(matches!(
        f.signal(A, ANY, OWNED),
        Lv2Dispatch::Immediate { code: 0, .. }
    ));
    timed_out(f.host.expire_wait(reason, B, GuestTicks::ZERO));
    assert!(f.lwmutex_waiters().is_empty());
}
