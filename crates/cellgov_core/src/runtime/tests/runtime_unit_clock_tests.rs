//! Each unit's own guest clock: what a step costs it, which unit runs
//! next, and the tick a parked unit resumes at.

use super::*;

use cellgov_ps3_abi::lv2::syscall::TIMER_USLEEP;
use std::cell::RefCell;
use std::rc::Rc;

/// Charges `cost` ticks a step for `steps` steps, and records the tick
/// at which each step starts.
#[derive(Clone)]
struct TimedUnit {
    id: UnitId,
    cost: u64,
    steps: u64,
    seen: Rc<RefCell<Vec<u64>>>,
}

impl ExecutionUnit for TimedUnit {
    type Snapshot = usize;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.seen.borrow().len() as u64 >= self.steps {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        _budget: Budget,
        ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.seen.borrow_mut().push(ctx.current_tick().raw());
        let done = self.seen.borrow().len() as u64 >= self.steps;
        ExecutionStepResult {
            yield_reason: if done {
                YieldReason::Finished
            } else {
                YieldReason::BudgetExhausted
            },
            consumed_cost: InstructionCost::new(self.cost),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> usize {
        self.seen.borrow().len()
    }
}

fn timed(rt: &mut Runtime, cost: u64, steps: u64) -> (UnitId, Rc<RefCell<Vec<u64>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let unit_seen = Rc::clone(&seen);
    let id = rt.registry_mut().register_with(move |id| TimedUnit {
        id,
        cost,
        steps,
        seen: unit_seen,
    });
    (id, seen)
}

fn run_out(rt: &mut Runtime) -> Vec<UnitId> {
    let mut order = Vec::new();
    for _ in 0..1_000 {
        let Ok(step) = rt.step() else { break };
        order.push(step.unit);
        rt.commit_step(&step.result, &step.effects).unwrap();
    }
    order
}

#[test]
fn two_busy_units_each_see_only_their_own_cost() {
    let mut rt = build(64, 1, 1_000);
    let (_, slow) = timed(&mut rt, 100, 4);
    let (_, fast) = timed(&mut rt, 7, 30);
    run_out(&mut rt);

    assert_eq!(*slow.borrow(), [0, 100, 200, 300]);
    let expected: Vec<u64> = (0..30).map(|n| n * 7).collect();
    assert_eq!(*fast.borrow(), expected);
}

#[test]
fn the_unit_with_the_lowest_clock_runs_next_and_a_tie_goes_to_the_lower_id() {
    let mut rt = build(64, 1, 1_000);
    let (slow, _) = timed(&mut rt, 100, 2);
    let (fast, _) = timed(&mut rt, 7, 16);
    let order = run_out(&mut rt);

    // Both start at 0, so the lower id runs first and reaches 100. The
    // other then runs until its clock passes 100: fifteen steps take it
    // to 105.
    let mut expected = vec![slow];
    expected.extend(std::iter::repeat_n(fast, 15));
    expected.push(slow);
    expected.push(fast);
    assert_eq!(order, expected);
}

/// Sleeps one microsecond, then records the tick it resumes at.
#[derive(Clone)]
struct Sleeper {
    id: UnitId,
    steps: Cell<u64>,
    resumed_at: Rc<Cell<Option<u64>>>,
}

impl ExecutionUnit for Sleeper {
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
        _budget: Budget,
        ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        if n == 1 {
            let mut args = [0u64; 9];
            args[0] = TIMER_USLEEP;
            args[1] = 1;
            return ExecutionStepResult {
                yield_reason: YieldReason::Syscall,
                consumed_cost: InstructionCost::new(100),
                local_diagnostics: LocalDiagnostics::with_pc(0x1000),
                fault: None,
                syscall_args: Some(args),
            };
        }
        self.resumed_at.set(Some(ctx.current_tick().raw()));
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[test]
fn a_sleeper_resumes_at_its_deadline_while_a_busy_peer_runs() {
    let mut rt = build(64, 1, 2_000);
    let resumed_at = Rc::new(Cell::new(None));
    let resumed = Rc::clone(&resumed_at);
    rt.registry_mut().register_with(move |id| Sleeper {
        id,
        steps: Cell::new(0),
        resumed_at: resumed,
    });
    // A 10-tick peer step does not divide the 3192-tick sleep, so a wake
    // taken at a peer's step boundary would land late.
    let _ = timed(&mut rt, 10, 1_000);
    run_out(&mut rt);

    // The sleep starts at the sleeper's own clock, 100, and lasts one
    // microsecond.
    assert_eq!(resumed_at.get(), Some(100 + cellgov_time::TICKS_PER_USEC));
}

/// The line the reservation waiter holds and the putter lands on.
const LINE: u64 = 0x80;

/// Reserves [`LINE`] and parks on an event in a 10-tick step, then
/// records the tick it resumes at.
#[derive(Clone)]
struct ReservationWaiter {
    id: UnitId,
    steps: Cell<u64>,
    resumed_at: Rc<Cell<Option<u64>>>,
}

impl ExecutionUnit for ReservationWaiter {
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

    fn channel_stall(&self) -> Option<cellgov_exec::ChannelStall> {
        (self.steps.get() == 1).then_some(cellgov_exec::ChannelStall {
            channel: cellgov_ps3_abi::hw::spu::SPU_RD_EVENT_STAT,
            wake: cellgov_exec::StallWake::Event,
        })
    }

    fn local_reservation(&self) -> Option<u64> {
        (self.steps.get() == 1).then_some(LINE)
    }

    fn run_until_yield(
        &mut self,
        _budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        if n == 1 {
            effects.push(Effect::ReservationAcquire {
                line_addr: LINE,
                source: self.id,
            });
            return ExecutionStepResult {
                yield_reason: YieldReason::ChannelStall,
                consumed_cost: InstructionCost::new(10),
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
            };
        }
        self.resumed_at.set(Some(ctx.current_tick().raw()));
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// Queues a put onto [`LINE`] in a 10-tick step and waits for it, then
/// finishes.
#[derive(Clone)]
struct LinePutter {
    id: UnitId,
    steps: Cell<u64>,
}

impl ExecutionUnit for LinePutter {
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
        _budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        if n == 1 {
            let source = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0), 4).unwrap();
            let destination =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(LINE), 4).unwrap();
            let request = cellgov_dma::DmaRequest::new(
                cellgov_dma::DmaDirection::Put,
                source,
                destination,
                self.id,
            )
            .unwrap();
            effects.push(Effect::DmaEnqueue {
                request,
                payload: None,
            });
            return ExecutionStepResult {
                yield_reason: YieldReason::DmaWait,
                consumed_cost: InstructionCost::new(10),
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
            };
        }
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// Reads the first byte of [`LINE`] at the start of each 20-tick step,
/// and records the tick and the byte.
#[derive(Clone)]
struct LineReader {
    id: UnitId,
    seen: Rc<RefCell<Vec<(u64, u8)>>>,
}

impl ExecutionUnit for LineReader {
    type Snapshot = usize;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.seen.borrow().len() >= 4 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        _budget: Budget,
        ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(LINE), 1).unwrap();
        let byte = ctx.memory().read(range).map_or(0, |bytes| bytes[0]);
        self.seen
            .borrow_mut()
            .push((ctx.current_tick().raw(), byte));
        let done = self.seen.borrow().len() >= 4;
        ExecutionStepResult {
            yield_reason: if done {
                YieldReason::Finished
            } else {
                YieldReason::BudgetExhausted
            },
            consumed_cost: InstructionCost::new(20),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> usize {
        self.seen.borrow().len()
    }
}

#[test]
fn a_landing_waits_for_the_lowest_runnable_clock_to_reach_its_tick() {
    let mut rt = build(256, 1, 1_000);
    rt.place_bytes(
        crate::runtime::AddressSpaceId::BOOT,
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0), 1).unwrap(),
        &[0xAB],
    )
    .unwrap();
    rt.registry_mut().register_with(|id| LinePutter {
        id,
        steps: Cell::new(0),
    });
    let seen = Rc::new(RefCell::new(Vec::new()));
    let reader_seen = Rc::clone(&seen);
    rt.registry_mut().register_with(move |id| LineReader {
        id,
        seen: reader_seen,
    });
    // The peer's first commit, at tick 100, is past the landing at 42
    // while the reader's clock is still at 20.
    let _ = timed(&mut rt, 100, 2);
    run_out(&mut rt);

    // The putter queues the put at tick 10. The put lands at 42, so only
    // the reader's step that starts at 60 sees it.
    assert_eq!(*seen.borrow(), [(0, 0), (20, 0), (40, 0), (60, 0xAB)]);
}

#[test]
fn a_waiter_whose_reservation_a_landing_clears_resumes_at_the_landing() {
    let mut rt = build(256, 1, 1_000);
    let resumed_at = Rc::new(Cell::new(None));
    let resumed = Rc::clone(&resumed_at);
    rt.registry_mut()
        .register_with(move |id| ReservationWaiter {
            id,
            steps: Cell::new(0),
            resumed_at: resumed,
        });
    rt.registry_mut().register_with(|id| LinePutter {
        id,
        steps: Cell::new(0),
    });
    // The peer's 100-tick step is the one whose commit finds the landing
    // due, at a tick well past it.
    let _ = timed(&mut rt, 100, 4);
    run_out(&mut rt);

    // The putter queues the put at its tick 10, and the put lands 32
    // ticks later.
    let landing = 10 + crate::DEFAULT_DMA_LATENCY_TICKS.raw();
    assert_eq!(resumed_at.get(), Some(landing));
}
