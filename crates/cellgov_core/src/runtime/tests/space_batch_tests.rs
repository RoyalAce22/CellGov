use std::cell::RefCell;
use std::rc::Rc;

use cellgov_effects::{Effect, FaultKind, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_sync::ReservedLine;
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::state::Runtime;

const S1: AddressSpaceId = AddressSpaceId::new(1);

fn build() -> Runtime {
    Runtime::new(GuestMemory::new(16), Budget::new(4), 100)
}

fn read4(mem: &GuestMemory, addr: u64) -> Option<Vec<u8>> {
    mem.read(ByteRange::new(GuestAddr::new(addr), 4).unwrap())
        .map(|b| b.to_vec())
}

/// Emits one 4-byte `SharedWriteIntent` and yields `reason`.
#[derive(Clone)]
struct OnceWriter {
    id: UnitId,
    addr: u64,
    value: u8,
    reason: YieldReason,
    done: bool,
}

impl OnceWriter {
    fn finished(id: UnitId, addr: u64, value: u8) -> Self {
        Self {
            id,
            addr,
            value,
            reason: YieldReason::Finished,
            done: false,
        }
    }

    fn faulting(id: UnitId, addr: u64, value: u8) -> Self {
        Self {
            id,
            addr,
            value,
            reason: YieldReason::Fault,
            done: false,
        }
    }
}

impl ExecutionUnit for OnceWriter {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.done {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.done = true;
        let range = ByteRange::new(GuestAddr::new(self.addr), 4).unwrap();
        effects.push(Effect::shared_write(
            range,
            WritePayload::new(vec![self.value; 4]),
            self.id,
            GuestTicks::ZERO,
        ));
        ExecutionStepResult {
            yield_reason: self.reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: if self.reason == YieldReason::Fault {
                Some(FaultKind::Guest(0))
            } else {
                None
            },
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

/// Emits nothing; records every `invalidate_code` call it receives.
#[derive(Clone)]
struct InvalidationRecorder {
    id: UnitId,
    calls: Rc<RefCell<Vec<(u64, u64)>>>,
    done: bool,
}

impl ExecutionUnit for InvalidationRecorder {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.done {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.done = true;
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn invalidate_code(&mut self, addr: u64, len: u64) {
        self.calls.borrow_mut().push((addr, len));
    }

    fn snapshot(&self) {}
}

#[test]
fn a_fault_discarded_batch_does_not_clear_sibling_view_reservations() {
    let mut rt = build();
    rt.create_address_space(S1).unwrap();
    rt.register_shared_mapping(11, 0x40, &[(AddressSpaceId::BOOT, 0x2000), (S1, 0x3000)])
        .unwrap();
    rt.registry_mut()
        .register_with(|id| OnceWriter::faulting(id, 0x3010, 0xC3));
    rt.assign_unit_space(UnitId::new(0), S1).unwrap();
    // Space-0 holder on the shared line through its own view; the
    // discarded child-space store must not sweep it.
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(9), ReservedLine::containing(0x2010));

    let s = rt.step().unwrap();
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
    assert!(outcome.fault_discarded);
    assert_eq!(
        outcome.reservations_cleared, 0,
        "a discarded batch must not clear reservations in any space",
    );
    assert!(
        rt.reservations().is_held_by(UnitId::new(9)),
        "fault-discards-all: the sibling-view reservation survived nothing being written",
    );
    assert_eq!(read4(rt.memory(), 0x2010).unwrap(), vec![0; 4]);
}

#[test]
fn pending_rsx_effects_defer_past_a_child_space_batch() {
    let mut rt = build();
    rt.create_address_space(S1).unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .install_region(0, 16, "child", PageSize::Page64K)
        .unwrap();
    rt.registry_mut()
        .register_with(|id| OnceWriter::finished(id, 0, 0xAB));
    rt.registry_mut()
        .register_with(|id| OnceWriter::finished(id, 4, 0xCD));
    rt.assign_unit_space(UnitId::new(0), S1).unwrap();
    // Deferred advance-pass effect targeting a space-0 address that
    // the child space also maps: committing it with the child batch
    // would land it in the wrong memory.
    rt.pending_rsx_effects.push(Effect::shared_write(
        ByteRange::new(GuestAddr::new(0x8), 4).unwrap(),
        WritePayload::new(vec![0xEE; 4]),
        UnitId::new(1),
        GuestTicks::ZERO,
    ));

    // Child-space batch: only the child's own write commits.
    let s = rt.step().unwrap();
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(
        read4(rt.space_memory(S1).unwrap(), 0x8).unwrap(),
        vec![0; 4],
        "the deferred space-0 effect leaked into the child space",
    );
    assert_eq!(rt.pending_rsx_effects.len(), 1);

    // Next space-0 batch picks it up.
    let s = rt.step().unwrap();
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(outcome.writes_committed, 2);
    assert_eq!(read4(rt.memory(), 0x8).unwrap(), vec![0xEE; 4]);
    assert!(rt.pending_rsx_effects.is_empty());
}

#[test]
fn a_shared_view_write_invalidates_code_at_sibling_alias_ranges() {
    let mut rt = build();
    rt.create_address_space(S1).unwrap();
    rt.register_shared_mapping(11, 0x40, &[(AddressSpaceId::BOOT, 0x2000), (S1, 0x3000)])
        .unwrap();
    rt.registry_mut()
        .register_with(|id| OnceWriter::finished(id, 0x2010, 0x5A));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let calls_in_unit = Rc::clone(&calls);
    rt.registry_mut()
        .register_with(move |id| InvalidationRecorder {
            id,
            calls: calls_in_unit,
            done: false,
        });

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(
        *calls.borrow(),
        vec![(0x2010, 4), (0x3010, 4)],
        "the fanout-replicated sibling range must be invalidated too",
    );
}

#[test]
fn a_child_space_write_at_the_put_register_address_leaves_the_cursor_alone() {
    use crate::rsx::control_register::PUT_ADDR;
    let mut rt = build();
    let mmio_base = u64::from(PUT_ADDR) & !0xFFF;
    rt.memory_mut()
        .install_region(mmio_base, 0x1000, "mmio", PageSize::Page64K)
        .unwrap();
    rt.memory_mut()
        .apply_commit(
            ByteRange::new(GuestAddr::new(u64::from(PUT_ADDR)), 4).unwrap(),
            &0x1234u32.to_be_bytes(),
        )
        .unwrap();
    rt.create_address_space(S1).unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .install_region(mmio_base, 0x1000, "child", PageSize::Page64K)
        .unwrap();
    rt.registry_mut()
        .register_with(|id| OnceWriter::finished(id, u64::from(PUT_ADDR), 0x77));
    rt.assign_unit_space(UnitId::new(0), S1).unwrap();
    rt.set_rsx_mirror_writes(true);

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    // Equal numeric addresses in different spaces never alias: the
    // child wrote its own memory, not the RSX control register.
    assert_eq!(rt.rsx_cursor().put(), 0);
    assert_eq!(
        read4(rt.memory(), u64::from(PUT_ADDR)).unwrap(),
        0x1234u32.to_be_bytes().to_vec(),
    );
    assert_eq!(
        read4(rt.space_memory(S1).unwrap(), u64::from(PUT_ADDR)).unwrap(),
        vec![0x77; 4],
    );
}
