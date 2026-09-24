//! The explorer's observable folds every unit's private memory beside
//! the committed memory.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

use crate::runtime::state::Runtime;

/// A unit that never runs and reports the private memory it was built
/// with, or none.
#[derive(Clone)]
struct PrivateMemoryUnit {
    id: UnitId,
    local: Option<Vec<u8>>,
}

impl ExecutionUnit for PrivateMemoryUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Finished
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn local_memory_hash(&self) -> Option<u64> {
        self.local.as_ref().map(|bytes| {
            let mut hasher = cellgov_mem::Fnv1aHasher::new();
            hasher.write(bytes);
            hasher.finish()
        })
    }

    fn snapshot(&self) {}
}

fn runtime_with(locals: &[Option<Vec<u8>>]) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(1), 10);
    for local in locals {
        let local = local.clone();
        rt.register_unit_with(move |id| PrivateMemoryUnit { id, local });
    }
    rt
}

#[test]
fn units_with_no_private_memory_leave_the_observable_at_the_committed_hash() {
    let rt = runtime_with(&[None, None]);
    assert_eq!(rt.observable_hash(), rt.committed_memory_hash());
}

#[test]
fn a_unit_with_private_memory_moves_the_observable_off_the_committed_hash() {
    let rt = runtime_with(&[None, Some(vec![0u8; 16])]);
    assert_ne!(rt.observable_hash(), rt.committed_memory_hash());
}

#[test]
fn a_private_byte_apart_is_a_different_observable() {
    let a = runtime_with(&[Some(vec![0u8; 16])]);
    let b = runtime_with(&[Some({
        let mut bytes = vec![0u8; 16];
        bytes[7] = 1;
        bytes
    })]);
    assert_eq!(a.committed_memory_hash(), b.committed_memory_hash());
    assert_ne!(a.observable_hash(), b.observable_hash());
}

#[test]
fn swapping_two_units_private_memories_changes_the_observable() {
    let a = runtime_with(&[Some(vec![1u8; 8]), Some(vec![2u8; 8])]);
    let b = runtime_with(&[Some(vec![2u8; 8]), Some(vec![1u8; 8])]);
    assert_ne!(a.observable_hash(), b.observable_hash());
}

#[test]
fn observable_hash_wire_format_golden() {
    let rt = runtime_with(&[None, Some(vec![0xA5u8; 4])]);
    let mut expected = cellgov_mem::Fnv1aHasher::new();
    expected.write(&rt.committed_memory_hash().to_le_bytes());
    expected.write(&1u64.to_le_bytes());
    let mut local = cellgov_mem::Fnv1aHasher::new();
    local.write(&[0xA5u8; 4]);
    expected.write(&local.finish().to_le_bytes());
    assert_eq!(rt.observable_hash(), expected.finish());
    assert_eq!(
        rt.observable_hash(),
        2_787_682_204_781_940_856,
        "literal pin; rewrite it by hand with the new value in the same commit that moves the stream",
    );
}
