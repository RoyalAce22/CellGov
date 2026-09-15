//! What the runtime reports to an installed `RuntimeTap`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, MemError, PageSize, Region, RegionAccess};
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use super::RuntimeTap;
use crate::commit::CommitError;
use crate::runtime::{AddressSpaceId, Runtime};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    Write(u64, Vec<u8>),
    /// The step number and the byte at address 0 the step saw.
    Step(u64, u8),
}

struct Recorder(Rc<RefCell<Vec<Seen>>>);

impl RuntimeTap for Recorder {
    fn write(&mut self, addr: u64, bytes: &[u8]) {
        self.0.borrow_mut().push(Seen::Write(addr, bytes.to_vec()));
    }

    fn step(&mut self, step: u64, memory: &GuestMemory) {
        let byte = memory.as_bytes()[0];
        self.0.borrow_mut().push(Seen::Step(step, byte));
    }
}

/// Writes its step number to 4 bytes at 0, and faults on step `fault_on`.
#[derive(Clone)]
struct Writer {
    id: UnitId,
    steps: Cell<u64>,
    fault_on: u64,
}

impl ExecutionUnit for Writer {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        effects.push(Effect::shared_write(
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            WritePayload::new(vec![n as u8; 4]),
            self.id,
            GuestTicks::ZERO,
        ));
        let faulted = n == self.fault_on;
        ExecutionStepResult {
            yield_reason: if faulted {
                YieldReason::Fault
            } else {
                YieldReason::BudgetExhausted
            },
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

fn tapped(fault_on: u64) -> (Runtime, Rc<RefCell<Vec<Seen>>>) {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(1), 100);
    let seen = Rc::new(RefCell::new(Vec::new()));
    rt.set_tap(Box::new(Recorder(Rc::clone(&seen))));
    rt.registry_mut().register_with(|id| Writer {
        id,
        steps: Cell::new(0),
        fault_on,
    });
    (rt, seen)
}

fn run(rt: &mut Runtime, steps: usize) {
    for _ in 0..steps {
        let s = rt.step().unwrap();
        rt.commit_step(&s.result, &s.effects).unwrap();
    }
}

#[test]
fn each_step_is_reported_before_its_batch_commits() {
    let (mut rt, seen) = tapped(0);
    run(&mut rt, 2);
    assert_eq!(
        *seen.borrow(),
        vec![
            Seen::Step(1, 0),
            Seen::Write(0, vec![1; 4]),
            Seen::Step(2, 1),
            Seen::Write(0, vec![2; 4]),
        ]
    );
}

#[test]
fn a_faulted_batch_reports_no_write() {
    let (mut rt, seen) = tapped(2);
    run(&mut rt, 3);
    let writes: Vec<Seen> = seen
        .borrow()
        .iter()
        .filter(|s| matches!(s, Seen::Write(..)))
        .cloned()
        .collect();
    assert_eq!(
        writes,
        vec![Seen::Write(0, vec![1; 4]), Seen::Write(0, vec![3; 4])]
    );
}

#[test]
fn a_host_write_is_reported() {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(1), 100);
    let seen = Rc::new(RefCell::new(Vec::new()));
    rt.set_tap(Box::new(Recorder(Rc::clone(&seen))));
    rt.place_bytes(
        AddressSpaceId::BOOT,
        ByteRange::new(GuestAddr::new(0x10), 2).unwrap(),
        &[0xAB, 0xCD],
    )
    .unwrap();
    assert_eq!(*seen.borrow(), vec![Seen::Write(0x10, vec![0xAB, 0xCD])]);
}

#[test]
fn a_refused_host_write_is_not_reported() {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(1), 100);
    let seen = Rc::new(RefCell::new(Vec::new()));
    rt.set_tap(Box::new(Recorder(Rc::clone(&seen))));
    let refused = rt.place_bytes(
        AddressSpaceId::BOOT,
        ByteRange::new(GuestAddr::new(0x1000), 2).unwrap(),
        &[0xAB, 0xCD],
    );
    assert!(matches!(refused, Err(MemError::Unmapped(_))), "{refused:?}");
    assert!(seen.borrow().is_empty());
}

const RESERVED: u64 = 0x1000;

/// A flat region at 0 and a reserved-zero region at `RESERVED`.
fn with_reserved_region() -> GuestMemory {
    GuestMemory::from_regions(vec![
        Region::new(0, 0x100, "flat", PageSize::Page64K),
        Region::with_access(
            RESERVED,
            0x100,
            "reserved",
            PageSize::Page4K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap()
}

/// Reads 4 bytes of the reserved region at every step.
struct ReservedReader;

impl RuntimeTap for ReservedReader {
    fn step(&mut self, _step: u64, memory: &GuestMemory) {
        let range = ByteRange::new(GuestAddr::new(RESERVED), 4).unwrap();
        assert_eq!(memory.read(range), Some(&[0u8; 4][..]));
    }
}

#[test]
fn a_reserved_read_by_the_tap_is_not_a_provisional_read() {
    let mut rt = Runtime::new(with_reserved_region(), Budget::new(1), 100);
    rt.set_tap(Box::new(ReservedReader));
    rt.registry_mut().register_with(|id| Writer {
        id,
        steps: Cell::new(0),
        fault_on: 0,
    });
    run(&mut rt, 2);
    assert_eq!(rt.memory().provisional_read_count(), 0);
    let range = ByteRange::new(GuestAddr::new(RESERVED), 4).unwrap();
    assert!(rt.memory().read(range).is_some());
    assert_eq!(
        rt.memory().provisional_read_count(),
        1,
        "logging is back on once the tap returns"
    );
}

/// Writes 4 bytes at 0, then 4 bytes at `RESERVED`, in one batch.
#[derive(Clone)]
struct IntoReserved {
    id: UnitId,
}

impl ExecutionUnit for IntoReserved {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        for addr in [0, RESERVED] {
            effects.push(Effect::shared_write(
                ByteRange::new(GuestAddr::new(addr), 4).unwrap(),
                WritePayload::new(vec![0xEE; 4]),
                self.id,
                GuestTicks::ZERO,
            ));
        }
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        0
    }
}

#[test]
fn a_batch_the_drain_refuses_reports_none_of_its_writes() {
    let mut rt = Runtime::new(with_reserved_region(), Budget::new(1), 100);
    let seen = Rc::new(RefCell::new(Vec::new()));
    rt.set_tap(Box::new(Recorder(Rc::clone(&seen))));
    rt.registry_mut().register_with(|id| IntoReserved { id });
    let s = rt.step().unwrap();
    let refused = rt.commit_step(&s.result, &s.effects);
    assert!(
        matches!(
            refused,
            Err(CommitError::Memory(MemError::ReservedWrite {
                addr: RESERVED,
                ..
            }))
        ),
        "{refused:?}"
    );
    assert_eq!(*seen.borrow(), vec![Seen::Step(1, 0)]);
}
