//! Timer-sleep park, deadline firing at commit boundaries, and the
//! all-blocked time-warp over the timer-wake queue.

use super::*;

use cellgov_ps3_abi::syscall::{PROCESS_EXIT, TIMER_SLEEP, TIMER_USLEEP};

/// Issues one timer syscall, then records the delivered r3 and
/// finishes on its next scheduling.
#[derive(Clone)]
struct SleepingUnit {
    id: UnitId,
    num: u64,
    arg: u64,
    steps: Cell<u64>,
    observed_r3: Cell<Option<u64>>,
}

impl SleepingUnit {
    fn new(id: UnitId, num: u64, arg: u64) -> Self {
        Self {
            id,
            num,
            arg,
            steps: Cell::new(0),
            observed_r3: Cell::new(None),
        }
    }
}

impl ExecutionUnit for SleepingUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        if n == 1 {
            let mut args = [0u64; 9];
            args[0] = self.num;
            args[1] = self.arg;
            ExecutionStepResult {
                yield_reason: YieldReason::Syscall,
                consumed_cost: InstructionCost::new(budget.raw()),
                local_diagnostics: LocalDiagnostics::with_pc(0x1000),
                fault: None,
                syscall_args: Some(args),
            }
        } else {
            self.observed_r3.set(ctx.syscall_return());
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_cost: InstructionCost::new(1),
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
            }
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[test]
fn usleep_parks_caller_and_time_warp_wakes_at_deadline() {
    let budget = 16u64;
    let mut rt = build(4096, budget, 100);
    let unit_id = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, TIMER_USLEEP, 50));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    assert_eq!(
        rt.registry().effective_status(unit_id),
        Some(UnitStatus::Blocked),
        "usleep caller must leave the runnable set"
    );
    let park_time = rt.time();
    // 50 us = 50_000 ticks past the time at dispatch.
    let expected_deadline = GuestTicks::new(park_time.raw() + 50_000);

    // Everything is parked: the next step must time-warp to the
    // deadline and run the woken sleeper.
    let s2 = rt.step().unwrap();
    assert_eq!(s2.unit, unit_id);
    assert_eq!(
        rt.time().raw(),
        expected_deadline.raw() + 1,
        "clock must jump to the deadline before the woken unit's own \
         1-instruction step advances it"
    );
    rt.commit_step(&s2.result, &s2.effects).unwrap();

    let unit = rt.registry().get(unit_id).unwrap();
    let sleeper = unit.as_any().downcast_ref::<SleepingUnit>().unwrap();
    assert_eq!(
        sleeper.observed_r3.get(),
        Some(0),
        "woken sleeper must observe r3 = 0 (CELL_OK)"
    );
}

#[test]
fn sleep_seconds_scale_to_microseconds() {
    let budget = 16u64;
    let mut rt = build(4096, budget, 100);
    rt.registry_mut()
        .register_with(|id| SleepingUnit::new(id, TIMER_SLEEP, 2));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    let park_time = rt.time();

    rt.step().unwrap();
    assert_eq!(
        rt.time().raw(),
        // 2 s = 2_000_000 us = 2e9 ticks, plus the woken step's cost.
        park_time.raw() + 2_000_000_000 + 1,
        "sys_timer_sleep seconds must scale through microseconds to ticks"
    );
}

#[test]
fn usleep_zero_is_a_yield_not_a_park() {
    let budget = 16u64;
    let mut rt = build(4096, budget, 100);
    let unit_id = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, TIMER_USLEEP, 0));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    assert_ne!(
        rt.registry().effective_status(unit_id),
        Some(UnitStatus::Blocked),
        "zero-interval sleep must not park"
    );

    let s2 = rt.step().unwrap();
    assert_eq!(s2.unit, unit_id);
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    let unit = rt.registry().get(unit_id).unwrap();
    let sleeper = unit.as_any().downcast_ref::<SleepingUnit>().unwrap();
    assert_eq!(sleeper.observed_r3.get(), Some(0));
}

#[test]
fn timer_wake_fires_at_commit_boundary_while_others_run() {
    // Sleeper parks for 1 us (1000 ticks); a busy peer's steps
    // advance time past the deadline, so the wake must fire at a
    // commit boundary without any time-warp.
    let budget = 300u64;
    let mut rt = build(4096, budget, 100);
    let sleeper_id = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, TIMER_USLEEP, 1));
    let _busy_id = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 20));

    let mut woke = false;
    for _ in 0..12 {
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
        if rt.registry().effective_status(sleeper_id) == Some(UnitStatus::Runnable) {
            woke = true;
            break;
        }
    }
    assert!(
        woke,
        "sleeper must return to the runnable set once a peer's \
         retirement pushes guest time past its deadline"
    );
    assert!(
        rt.time().raw() >= 1000,
        "wake must not fire before the deadline"
    );
}

#[test]
fn usleep_wake_emits_timer_unit_woken_record() {
    use cellgov_trace::{TraceReader, TraceRecord, TracedWakeReason};

    let budget = 16u64;
    let mut rt = build(4096, budget, 100);
    let unit_id = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, TIMER_USLEEP, 50));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();

    let bytes = rt.trace().bytes().to_vec();
    let records: Vec<TraceRecord> = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .collect();
    let timer_woken = records.iter().any(|r| {
        matches!(
            r,
            TraceRecord::UnitWoken {
                unit,
                reason: TracedWakeReason::Timer,
            } if *unit == unit_id
        )
    });
    assert!(
        timer_woken,
        "a timer-driven wake must emit UnitWoken with reason Timer"
    );
}

#[test]
fn snapshot_restores_pending_timer_wake() {
    let budget = 16u64;
    let mut rt = build(4096, budget, 100);
    let unit_id = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, TIMER_USLEEP, 50));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    let hash_parked = rt.sync_state_hash();

    let snap = rt.snapshot();

    // Fire the wake, mutating queue + response table.
    rt.step().unwrap();
    assert_ne!(
        rt.sync_state_hash(),
        hash_parked,
        "firing the wake must change the sync state hash"
    );

    rt.restore_into(&snap);
    rt.set_scheduler(crate::scheduler::RoundRobinScheduler::new());
    assert_eq!(
        rt.sync_state_hash(),
        hash_parked,
        "restore must bring back the pending timer wake"
    );

    let s2 = rt.step().unwrap();
    assert_eq!(s2.unit, unit_id);
    rt.commit_step(&s2.result, &s2.effects).unwrap();
    let unit = rt.registry().get(unit_id).unwrap();
    let sleeper = unit.as_any().downcast_ref::<SleepingUnit>().unwrap();
    assert_eq!(sleeper.observed_r3.get(), Some(0));
}

/// Yields `BudgetExhausted` forever, recording the most recent
/// syscall return the runtime delivered.
#[derive(Clone)]
struct RecordingUnit {
    id: UnitId,
    observed_r3: Cell<Option<u64>>,
}

impl ExecutionUnit for RecordingUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        if let Some(code) = ctx.syscall_return() {
            self.observed_r3.set(Some(code));
        }
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

fn seed_waiter_thread(rt: &mut Runtime, unit: UnitId) {
    rt.lv2_host_mut().seed_primary_ppu_thread(
        unit,
        cellgov_lv2::PpuThreadAttrs {
            entry: 0,
            arg: 0,
            stack_base: 0,
            stack_size: 0,
            priority: 0,
            tls_base: 0,
        },
    );
}

#[test]
fn semaphore_timed_wait_expires_with_etimedout() {
    const SEMA_ID: u32 = 7;
    let budget = 300u64;
    let mut rt = build(4096, budget, 200);
    let waiter = rt.registry_mut().register_with(|id| RecordingUnit {
        id,
        observed_r3: Cell::new(None),
    });
    let _busy = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    seed_waiter_thread(&mut rt, waiter);
    rt.lv2_host_mut()
        .semaphores_mut()
        .create_with_id(SEMA_ID, 0, 8)
        .unwrap();

    // 1 us deadline = 1000 ticks; the busy peer's steps push time
    // past it, so the expiry fires at a commit boundary.
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::SemaphoreWait {
            id: SEMA_ID,
            timeout: 1,
        },
        waiter,
    );
    assert_eq!(
        rt.registry().effective_status(waiter),
        Some(UnitStatus::Blocked)
    );
    assert_eq!(rt.timer_wakes_pending(), 1);

    for _ in 0..12 {
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
    }
    assert_eq!(rt.timer_wakes_pending(), 0, "deadline must have fired");
    let unit = rt.registry().get(waiter).unwrap();
    let recorder = unit.as_any().downcast_ref::<RecordingUnit>().unwrap();
    assert_eq!(
        recorder.observed_r3.get(),
        Some(0x8001_000Bu64),
        "expired semaphore wait must deliver CELL_ETIMEDOUT"
    );
    assert!(
        rt.lv2_host()
            .semaphores()
            .lookup(SEMA_ID)
            .unwrap()
            .waiters()
            .is_empty(),
        "expired waiter must leave the semaphore's list"
    );
    assert_eq!(rt.lv2_host().observability().invariant_break_count, 0);
}

#[test]
fn post_before_deadline_cancels_timer_entry() {
    const SEMA_ID: u32 = 9;
    let budget = 300u64;
    let mut rt = build(4096, budget, 200);
    let waiter = rt.registry_mut().register_with(|id| RecordingUnit {
        id,
        observed_r3: Cell::new(None),
    });
    let poster = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    seed_waiter_thread(&mut rt, waiter);
    rt.lv2_host_mut()
        .semaphores_mut()
        .create_with_id(SEMA_ID, 0, 8)
        .unwrap();

    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::SemaphoreWait {
            id: SEMA_ID,
            timeout: 1_000_000,
        },
        waiter,
    );
    assert_eq!(rt.timer_wakes_pending(), 1);

    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::SemaphorePost {
            id: SEMA_ID,
            val: 1,
        },
        poster,
    );
    assert_eq!(
        rt.registry().effective_status(waiter),
        Some(UnitStatus::Runnable)
    );
    assert_eq!(
        rt.timer_wakes_pending(),
        0,
        "wake must cancel the pending deadline"
    );

    for _ in 0..8 {
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
    }
    let unit = rt.registry().get(waiter).unwrap();
    let recorder = unit.as_any().downcast_ref::<RecordingUnit>().unwrap();
    assert_eq!(
        recorder.observed_r3.get(),
        Some(0),
        "a wake that wins the race delivers success, not ETIMEDOUT"
    );
    assert_eq!(rt.lv2_host().observability().invariant_break_count, 0);
}

#[test]
fn process_exit_with_a_due_sleep_does_not_resurrect_the_sleeper() {
    // The sleeper's deadline (park time 2000 + 1000 ticks) falls due
    // inside the same commit that dispatches PROCESS_EXIT: the exiter's
    // own 2000-tick step pushes time to 4000 first. The exit sweep
    // finishes every unit and drops the sleeper's parked response; the
    // due timer entry must be dropped, not fired -- RPCS3
    // sys_process.cpp _sys_process_exit stops every thread, so a due
    // sleep never resumes its thread after exit.
    let budget = 2000u64;
    let mut rt = build(4096, budget, 100);
    let sleeper = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, TIMER_USLEEP, 1));
    let _exiter = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, PROCESS_EXIT, 0));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.timer_wakes_pending(), 1);

    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();

    assert_eq!(
        rt.registry().effective_status(sleeper),
        Some(UnitStatus::Finished),
        "a due sleep must not resurrect a unit the exit sweep finished"
    );
    assert_eq!(
        rt.timer_wakes_pending(),
        0,
        "the due entry is consumed, not leaked"
    );
    assert_eq!(rt.step().unwrap_err(), StepError::NoRunnableUnit);
}

#[test]
fn all_blocked_time_warp_expires_semaphore_wait_with_etimedout() {
    // The timed waiter is the only unit: expiry cannot ride a busy
    // peer's commit boundary and must come from the all-blocked
    // time-warp in Runtime::step.
    const SEMA_ID: u32 = 11;
    let budget = 16u64;
    let mut rt = build(4096, budget, 100);
    let waiter = rt.registry_mut().register_with(|id| RecordingUnit {
        id,
        observed_r3: Cell::new(None),
    });
    seed_waiter_thread(&mut rt, waiter);
    rt.lv2_host_mut()
        .semaphores_mut()
        .create_with_id(SEMA_ID, 0, 8)
        .unwrap();

    // 5 us deadline = 5000 ticks from time 0.
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::SemaphoreWait {
            id: SEMA_ID,
            timeout: 5,
        },
        waiter,
    );
    assert_eq!(
        rt.registry().effective_status(waiter),
        Some(UnitStatus::Blocked)
    );
    assert_eq!(rt.timer_wakes_pending(), 1);

    let s = rt.step().unwrap();
    assert_eq!(s.unit, waiter);
    assert_eq!(
        rt.time().raw(),
        5000 + budget,
        "clock must warp to the deadline before the woken waiter's own \
         step advances it"
    );
    rt.commit_step(&s.result, &s.effects).unwrap();

    let unit = rt.registry().get(waiter).unwrap();
    let recorder = unit.as_any().downcast_ref::<RecordingUnit>().unwrap();
    assert_eq!(
        recorder.observed_r3.get(),
        Some(0x8001_000Bu64),
        "warp-expired semaphore wait must deliver CELL_ETIMEDOUT"
    );
    assert_eq!(rt.timer_wakes_pending(), 0);
    assert!(
        rt.lv2_host()
            .semaphores()
            .lookup(SEMA_ID)
            .unwrap()
            .waiters()
            .is_empty(),
        "expired waiter must leave the semaphore's list"
    );
    assert_eq!(rt.lv2_host().observability().invariant_break_count, 0);
}

#[test]
fn sleep_seconds_argument_reads_only_the_low_32_bits() {
    // r3 = 1 << 32: the seconds parameter is 32-bit, so the kernel
    // sees 0 seconds -- a yield, not a 136-year park.
    let budget = 16u64;
    let mut rt = build(4096, budget, 100);
    let unit_id = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, TIMER_SLEEP, 1u64 << 32));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    assert_ne!(
        rt.registry().effective_status(unit_id),
        Some(UnitStatus::Blocked),
        "the upper register half must not park the caller"
    );
    assert_eq!(rt.timer_wakes_pending(), 0);
}

#[test]
fn process_exit_drops_pending_timer_wakes() {
    use cellgov_ps3_abi::syscall::PROCESS_EXIT;

    // The exiting step itself crosses the sleeper's deadline, so
    // without the exit-sweep cancel the same commit's timer firing
    // would wake a unit the sweep just finished.
    let budget = 2000u64;
    let mut rt = build(4096, budget, 100);
    let sleeper = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, TIMER_USLEEP, 1));
    let exiter = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, PROCESS_EXIT, 0));

    let s = rt.step().unwrap();
    assert_eq!(s.unit, sleeper);
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.timer_wakes_pending(), 1);

    let s2 = rt.step().unwrap();
    assert_eq!(s2.unit, exiter);
    rt.commit_step(&s2.result, &s2.effects).unwrap();

    assert_eq!(
        rt.timer_wakes_pending(),
        0,
        "process exit must drop every pending timer wake"
    );
    assert_eq!(
        rt.registry().effective_status(sleeper),
        Some(UnitStatus::Finished),
        "a due deadline must not resurrect a unit process exit finished"
    );
}

#[test]
fn contended_cond_expiry_reparks_without_phantom_timer_wake_record() {
    use cellgov_trace::{TraceReader, TraceRecord, TracedWakeReason};

    const MUTEX_ID: u32 = 21;
    const COND_ID: u32 = 22;
    let budget = 300u64;
    let mut rt = build(4096, budget, 200);
    let sleeper = rt.registry_mut().register_with(|id| RecordingUnit {
        id,
        observed_r3: Cell::new(None),
    });
    let holder = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 100));
    seed_waiter_thread(&mut rt, sleeper);
    rt.lv2_host_mut()
        .ppu_threads_mut()
        .create(
            holder,
            cellgov_lv2::PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    rt.lv2_host_mut()
        .mutexes_mut()
        .create_with_id(MUTEX_ID, cellgov_lv2::MutexAttrs::default())
        .unwrap();
    rt.lv2_host_mut()
        .conds_mut()
        .create_with_id(COND_ID, MUTEX_ID, cellgov_lv2::CondMutexKind::Mutex)
        .unwrap();

    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::MutexLock {
            mutex_id: MUTEX_ID,
            timeout: 0,
        },
        sleeper,
    );
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::CondWait {
            id: COND_ID,
            timeout: 1,
        },
        sleeper,
    );
    assert_eq!(
        rt.registry().effective_status(sleeper),
        Some(UnitStatus::Blocked)
    );
    assert_eq!(rt.timer_wakes_pending(), 1);
    // The peer takes the mutex the cond wait released, so the expiry
    // re-parks the sleeper on the mutex instead of waking it.
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::MutexLock {
            mutex_id: MUTEX_ID,
            timeout: 0,
        },
        holder,
    );

    for _ in 0..8 {
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
        if rt.timer_wakes_pending() == 0 {
            break;
        }
    }
    assert_eq!(rt.timer_wakes_pending(), 0, "deadline must have fired");
    assert_eq!(
        rt.registry().effective_status(sleeper),
        Some(UnitStatus::Blocked),
        "contended cond expiry re-parks on the mutex, no wake"
    );
    let bytes = rt.trace().bytes().to_vec();
    let phantom = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .any(|r| {
            matches!(
                r,
                TraceRecord::UnitWoken {
                    unit,
                    reason: TracedWakeReason::Timer,
                } if unit == sleeper
            )
        });
    assert!(
        !phantom,
        "a fired deadline that re-parked the unit must not record UnitWoken"
    );

    // The unlock hands the mutex over; the sleeper returns ETIMEDOUT.
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::MutexUnlock { mutex_id: MUTEX_ID },
        holder,
    );
    assert_eq!(
        rt.registry().effective_status(sleeper),
        Some(UnitStatus::Runnable)
    );
    for _ in 0..4 {
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
    }
    let unit = rt.registry().get(sleeper).unwrap();
    let recorder = unit.as_any().downcast_ref::<RecordingUnit>().unwrap();
    assert_eq!(
        recorder.observed_r3.get(),
        Some(0x8001_000Bu64),
        "cond timeout with contended mutex must still deliver CELL_ETIMEDOUT"
    );
    assert_eq!(rt.lv2_host().observability().invariant_break_count, 0);
}

#[test]
fn a_huge_usleep_deadline_saturates_at_the_tick_ceiling() {
    // u64::MAX usec * 1000 overflows u64; the conversion must clamp
    // to the tick ceiling (RPCS3 sys_timer.cpp sys_timer_usleep also
    // saturates its sleep-time arithmetic). Wrapping would fabricate
    // an in-the-past deadline and fire a spurious wake at this same
    // commit.
    let budget = 16u64;
    let mut rt = build(4096, budget, 100);
    let unit_id = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, TIMER_USLEEP, u64::MAX));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    assert_eq!(
        rt.registry().effective_status(unit_id),
        Some(UnitStatus::Blocked),
        "a wrapped deadline would already have fired at this commit"
    );
    assert_eq!(rt.timer_wakes_pending(), 1);
    assert_eq!(
        rt.timer_wakes.peek_deadline(),
        Some(GuestTicks::new(u64::MAX)),
        "deadline must clamp to the tick ceiling, not wrap"
    );
}

#[test]
fn a_due_wake_for_a_finished_unit_is_dropped_unfired() {
    use cellgov_trace::{TraceReader, TraceRecord, TracedWakeReason};

    // Exit-sweep residue reaching the firing pass directly: a due
    // entry whose unit is already Finished and whose parked response
    // is gone. Without the Finished guard the firing would resurrect
    // the unit (Runnable override replaces Finished) and log the
    // missing-response invariant break. RPCS3 sys_process.cpp
    // _sys_process_exit stops every thread, so a due sleep never
    // resumes its thread after exit.
    let budget = 2000u64;
    let mut rt = build(4096, budget, 100);
    let finished = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, TIMER_USLEEP, 1));
    let _busy = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 4));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(rt.timer_wakes_pending(), 1);
    rt.registry_mut()
        .set_status_override(finished, UnitStatus::Finished);
    let _ = rt.syscall_responses.try_take(finished);

    // The busy peer's 2000-tick step crosses the 1000-tick deadline.
    let s2 = rt.step().unwrap();
    rt.commit_step(&s2.result, &s2.effects).unwrap();

    assert_eq!(rt.timer_wakes_pending(), 0, "the due entry is consumed");
    assert_eq!(
        rt.registry().effective_status(finished),
        Some(UnitStatus::Finished),
        "a dropped wake must not resurrect a Finished unit"
    );
    assert_eq!(
        rt.lv2_host().observability().invariant_break_count,
        0,
        "the dropped wake must never reach the sync-wake resolver"
    );
    let bytes = rt.trace().bytes().to_vec();
    let phantom = TraceReader::new(&bytes)
        .map(|r| r.expect("decode"))
        .any(|r| {
            matches!(
                r,
                TraceRecord::UnitWoken {
                    unit,
                    reason: TracedWakeReason::Timer,
                } if unit == finished
            )
        });
    assert!(!phantom, "a dropped wake must not record UnitWoken");
}

#[test]
fn all_blocked_without_timer_or_dma_still_errors() {
    let budget = 16u64;
    let mut rt = build(4096, budget, 100);
    let unit_id = rt
        .registry_mut()
        .register_with(|id| SleepingUnit::new(id, TIMER_USLEEP, 50));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    // Cancel the timer entry out from under the parked unit; the
    // runtime now has a Blocked unit and no wake source.
    rt.timer_wakes_cancel_for_test(unit_id);
    let err = rt.step().unwrap_err();
    assert_eq!(err, StepError::AllBlocked);
}

#[test]
fn a_no_wake_expiry_warps_again_instead_of_reporting_all_blocked() {
    // The earliest deadline is a timed cond wait whose mutex the peer
    // holds: its expiry re-parks the waiter untimed and wakes nobody.
    // The time-warp must continue to the peer's later semaphore
    // deadline instead of reporting a terminal AllBlocked while a
    // live wake source remains queued.
    const MUTEX_ID: u32 = 31;
    const COND_ID: u32 = 32;
    const SEMA_ID: u32 = 33;
    let budget = 300u64;
    let mut rt = build(4096, budget, 200);
    let cond_waiter = rt.registry_mut().register_with(|id| RecordingUnit {
        id,
        observed_r3: Cell::new(None),
    });
    let holder = rt.registry_mut().register_with(|id| RecordingUnit {
        id,
        observed_r3: Cell::new(None),
    });
    seed_waiter_thread(&mut rt, cond_waiter);
    rt.lv2_host_mut()
        .ppu_threads_mut()
        .create(
            holder,
            cellgov_lv2::PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    rt.lv2_host_mut()
        .mutexes_mut()
        .create_with_id(MUTEX_ID, cellgov_lv2::MutexAttrs::default())
        .unwrap();
    rt.lv2_host_mut()
        .conds_mut()
        .create_with_id(COND_ID, MUTEX_ID, cellgov_lv2::CondMutexKind::Mutex)
        .unwrap();
    rt.lv2_host_mut()
        .semaphores_mut()
        .create_with_id(SEMA_ID, 0, 8)
        .unwrap();

    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::MutexLock {
            mutex_id: MUTEX_ID,
            timeout: 0,
        },
        cond_waiter,
    );
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::CondWait {
            id: COND_ID,
            timeout: 1,
        },
        cond_waiter,
    );
    // The peer takes the mutex the cond wait released, then parks on
    // a timed semaphore wait with a later deadline.
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::MutexLock {
            mutex_id: MUTEX_ID,
            timeout: 0,
        },
        holder,
    );
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::SemaphoreWait {
            id: SEMA_ID,
            timeout: 50,
        },
        holder,
    );
    assert_eq!(
        rt.registry().effective_status(cond_waiter),
        Some(UnitStatus::Blocked)
    );
    assert_eq!(
        rt.registry().effective_status(holder),
        Some(UnitStatus::Blocked)
    );
    assert_eq!(rt.timer_wakes_pending(), 2);

    let s = rt
        .step()
        .expect("warp must continue past a no-wake cond expiry to the semaphore deadline");
    assert_eq!(s.unit, holder, "the semaphore expiry wakes the peer");
    rt.commit_step(&s.result, &s.effects).unwrap();

    assert_eq!(
        rt.registry().effective_status(cond_waiter),
        Some(UnitStatus::Blocked),
        "contended cond expiry re-parks on the mutex, no wake"
    );
    let unit = rt.registry().get(holder).unwrap();
    let recorder = unit.as_any().downcast_ref::<RecordingUnit>().unwrap();
    assert_eq!(
        recorder.observed_r3.get(),
        Some(0x8001_000Bu64),
        "the second warp's semaphore expiry must deliver CELL_ETIMEDOUT"
    );
    assert_eq!(rt.timer_wakes_pending(), 0);
    assert_eq!(rt.lv2_host().observability().invariant_break_count, 0);
}
