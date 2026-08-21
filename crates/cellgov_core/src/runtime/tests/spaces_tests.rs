use std::cell::Cell;

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::PriorityClass;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_sync::ReservedLine;
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use super::*;

#[derive(Clone)]
struct AddrWriter {
    id: UnitId,
    addr: u64,
    value: u8,
    done: Cell<bool>,
}

impl AddrWriter {
    fn new(id: UnitId, addr: u64, value: u8) -> Self {
        Self {
            id,
            addr,
            value,
            done: Cell::new(false),
        }
    }
}

impl ExecutionUnit for AddrWriter {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.done.get() {
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
        self.done.set(true);
        let range = ByteRange::new(GuestAddr::new(self.addr), 4).unwrap();
        effects.push(Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::new(vec![self.value; 4]),
            ordering: PriorityClass::Normal,
            source: self.id,
            source_time: GuestTicks::ZERO,
        });
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

/// Emits `ReservationAcquire` on its first step and a matching
/// `ConditionalStore` on its second.
#[derive(Clone)]
struct AtomicWriter {
    id: UnitId,
    addr: u64,
    phase: Cell<u8>,
}

impl AtomicWriter {
    fn new(id: UnitId, addr: u64) -> Self {
        Self {
            id,
            addr,
            phase: Cell::new(0),
        }
    }
}

impl ExecutionUnit for AtomicWriter {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.phase.get() >= 2 {
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
        let phase = self.phase.get();
        self.phase.set(phase + 1);
        let yield_reason = if phase == 0 {
            effects.push(Effect::ReservationAcquire {
                line_addr: self.addr,
                source: self.id,
            });
            YieldReason::BudgetExhausted
        } else {
            let range = ByteRange::new(GuestAddr::new(self.addr), 4).unwrap();
            effects.push(Effect::ConditionalStore {
                range,
                bytes: WritePayload::new(vec![0xEE; 4]),
                ordering: PriorityClass::Normal,
                source: self.id,
                source_time: GuestTicks::ZERO,
            });
            YieldReason::Finished
        };
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

/// Stays runnable for `remaining` steps, emitting nothing.
#[derive(Clone)]
struct Spinner {
    id: UnitId,
    remaining: Cell<u32>,
}

impl Spinner {
    fn new(id: UnitId, steps: u32) -> Self {
        Self {
            id,
            remaining: Cell::new(steps),
        }
    }
}

impl ExecutionUnit for Spinner {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.remaining.get() == 0 {
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
        let left = self.remaining.get() - 1;
        self.remaining.set(left);
        ExecutionStepResult {
            yield_reason: if left == 0 {
                YieldReason::Finished
            } else {
                YieldReason::BudgetExhausted
            },
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

fn build(memory_size: usize) -> Runtime {
    Runtime::new(
        GuestMemory::new(memory_size),
        cellgov_time::Budget::new(4),
        100,
    )
}

fn read4(mem: &GuestMemory, addr: u64) -> Option<Vec<u8>> {
    mem.read(ByteRange::new(GuestAddr::new(addr), 4).unwrap())
        .map(|b| b.to_vec())
}

const S1: AddressSpaceId = AddressSpaceId::new(1);

#[test]
fn child_space_write_stays_out_of_space_zero() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .install_region(0, 16, "child", PageSize::Page64K)
        .unwrap();
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0, 0xAB));
    rt.assign_unit_space(UnitId::new(0), S1).unwrap();

    let s = rt.step().unwrap();
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(outcome.writes_committed, 1);
    assert_eq!(
        read4(rt.space_memory(S1).unwrap(), 0).unwrap(),
        vec![0xAB; 4]
    );
    assert_eq!(read4(rt.memory(), 0).unwrap(), vec![0; 4]);
}

#[test]
fn spaces_have_disjoint_layouts() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .install_region(0x1000, 0x40, "child", PageSize::Page64K)
        .unwrap();
    assert!(read4(rt.memory(), 0x1000).is_none());
    assert!(read4(rt.space_memory(S1).unwrap(), 0).is_none());
    assert!(read4(rt.space_memory(S1).unwrap(), 0x1000).is_some());
}

#[test]
fn create_rejects_space_zero_and_duplicates() {
    let mut rt = build(16);
    assert_eq!(
        rt.create_address_space(AddressSpaceId::BOOT),
        Err(SpaceError::SpaceExists(0))
    );
    rt.create_address_space(S1).unwrap();
    assert_eq!(rt.create_address_space(S1), Err(SpaceError::SpaceExists(1)));
}

#[test]
fn assign_rejects_unknown_space_and_boot_untags() {
    let mut rt = build(16);
    assert_eq!(
        rt.assign_unit_space(UnitId::new(0), S1),
        Err(SpaceError::UnknownSpace(1))
    );
    rt.create_address_space(S1).unwrap();
    rt.assign_unit_space(UnitId::new(0), S1).unwrap();
    assert_eq!(rt.unit_space(UnitId::new(0)), S1);
    rt.assign_unit_space(UnitId::new(0), AddressSpaceId::BOOT)
        .unwrap();
    assert_eq!(rt.unit_space(UnitId::new(0)), AddressSpaceId::BOOT);
}

#[test]
fn shared_mapping_replicates_writes_both_directions() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    rt.register_shared_mapping(
        0x8006_0100_0000_0010,
        0x40,
        &[(AddressSpaceId::BOOT, 0x2000), (S1, 0x3000)],
    )
    .unwrap();

    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0x2010, 0x5A));
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0x3020, 0xC3));
    rt.assign_unit_space(UnitId::new(1), S1).unwrap();

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();

    // Space-0 write visible in space 1's view.
    assert_eq!(
        read4(rt.space_memory(S1).unwrap(), 0x3010).unwrap(),
        vec![0x5A; 4]
    );
    // Space-1 write visible in space 0's view.
    assert_eq!(read4(rt.memory(), 0x2020).unwrap(), vec![0xC3; 4]);
}

#[test]
fn shared_mapping_rejects_duplicate_key_and_unknown_space() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    rt.register_shared_mapping(7, 0x40, &[(AddressSpaceId::BOOT, 0x2000), (S1, 0x3000)])
        .unwrap();
    assert_eq!(
        rt.register_shared_mapping(7, 0x40, &[(S1, 0x4000)]),
        Err(SpaceError::KeyExists(7))
    );
    assert_eq!(
        rt.register_shared_mapping(8, 0x40, &[(AddressSpaceId::new(9), 0x4000)]),
        Err(SpaceError::UnknownSpace(9))
    );
}

#[test]
fn sync_state_hash_moves_when_a_unit_is_tagged() {
    let mut rt = build(16);
    let pre = rt.sync_state_hash();
    rt.create_address_space(S1).unwrap();
    rt.assign_unit_space(UnitId::new(0), S1).unwrap();
    assert_ne!(pre, rt.sync_state_hash());
    // Untagging alone does not restore the baseline: the empty child
    // space still folds via the mapping metadata gate only when
    // non-empty state remains -- an extra space with no tags still
    // counts as non-empty table.
    rt.assign_unit_space(UnitId::new(0), AddressSpaceId::BOOT)
        .unwrap();
    assert_ne!(pre, rt.sync_state_hash());
}

#[test]
fn committed_memory_hash_equals_space_zero_hash_when_no_children() {
    let rt = build(16);
    assert_eq!(rt.committed_memory_hash(), rt.memory().content_hash());
}

#[test]
fn committed_memory_hash_moves_with_a_child_space() {
    let mut rt = build(16);
    let pre = rt.committed_memory_hash();
    rt.create_address_space(S1).unwrap();
    assert_ne!(pre, rt.committed_memory_hash());
}

#[test]
fn snapshot_restore_carries_spaces() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .install_region(0, 16, "child", PageSize::Page64K)
        .unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .apply_commit(ByteRange::new(GuestAddr::new(0), 4).unwrap(), &[9, 9, 9, 9])
        .unwrap();
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0, 1));
    rt.assign_unit_space(UnitId::new(0), S1).unwrap();
    rt.space_reservations_mut(S1)
        .unwrap()
        .insert_or_replace(UnitId::new(0), ReservedLine::containing(0));

    let snap = rt.snapshot();
    rt.space_memory_mut(S1)
        .unwrap()
        .apply_commit(ByteRange::new(GuestAddr::new(0), 4).unwrap(), &[7, 7, 7, 7])
        .unwrap();
    rt.space_reservations_mut(S1)
        .unwrap()
        .remove_if_present(UnitId::new(0));
    rt.restore_into(&snap);
    assert_eq!(
        read4(rt.space_memory(S1).unwrap(), 0).unwrap(),
        vec![9, 9, 9, 9]
    );
    assert_eq!(rt.unit_space(UnitId::new(0)), S1);
    assert!(rt
        .space_reservations(S1)
        .unwrap()
        .is_held_by(UnitId::new(0)));
}

#[test]
fn child_space_write_leaves_space_zero_reservations_alone() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .install_region(0, 16, "child", PageSize::Page64K)
        .unwrap();
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0, 0xAB));
    rt.assign_unit_space(UnitId::new(0), S1).unwrap();
    // Space-0 holder on the same numeric line the child-space store
    // hits; the addresses name different memory.
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(9), ReservedLine::containing(0));

    let s = rt.step().unwrap();
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(outcome.reservations_cleared, 0);
    assert!(rt.reservations().is_held_by(UnitId::new(9)));
}

#[test]
fn space_zero_write_leaves_child_space_reservations_alone() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0, 0xAB));
    rt.space_reservations_mut(S1)
        .unwrap()
        .insert_or_replace(UnitId::new(9), ReservedLine::containing(0));

    let s = rt.step().unwrap();
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(outcome.reservations_cleared, 0);
    assert!(rt
        .space_reservations(S1)
        .unwrap()
        .is_held_by(UnitId::new(9)));
}

#[test]
fn conditional_store_sweep_is_space_scoped() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .install_region(0, 16, "child", PageSize::Page64K)
        .unwrap();
    rt.registry_mut()
        .register_with(|id| AtomicWriter::new(id, 0));
    rt.assign_unit_space(UnitId::new(0), S1).unwrap();
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(9), ReservedLine::containing(0));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert!(rt
        .space_reservations(S1)
        .unwrap()
        .is_held_by(UnitId::new(0)));

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    // Bytes landed in the child space only; the stwcx retired its own
    // entry and swept its own space, not space 0.
    assert_eq!(
        read4(rt.space_memory(S1).unwrap(), 0).unwrap(),
        vec![0xEE; 4]
    );
    assert_eq!(read4(rt.memory(), 0).unwrap(), vec![0; 4]);
    assert!(rt.space_reservations(S1).unwrap().is_empty());
    assert!(rt.reservations().is_held_by(UnitId::new(9)));
}

#[test]
fn shared_write_clears_sibling_view_reservations() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    rt.register_shared_mapping(11, 0x40, &[(AddressSpaceId::BOOT, 0x2000), (S1, 0x3000)])
        .unwrap();
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0x3010, 0xC3));
    rt.assign_unit_space(UnitId::new(0), S1).unwrap();
    // Space-0 holder on the shared line through its own view; the
    // child-space store reaches the same bytes.
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(9), ReservedLine::containing(0x2010));

    let s = rt.step().unwrap();
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(read4(rt.memory(), 0x2010).unwrap(), vec![0xC3; 4]);
    assert!(!rt.reservations().is_held_by(UnitId::new(9)));
    assert_eq!(outcome.reservations_cleared, 1);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "targets a shared view")]
fn conditional_store_on_shared_view_is_surfaced() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    rt.register_shared_mapping(11, 0x40, &[(AddressSpaceId::BOOT, 0x2000), (S1, 0x3000)])
        .unwrap();
    rt.registry_mut()
        .register_with(|id| AtomicWriter::new(id, 0x3010));
    rt.assign_unit_space(UnitId::new(0), S1).unwrap();

    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
}

#[test]
fn rotation_interleaves_cross_space_units() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    rt.registry_mut().register_with(|id| Spinner::new(id, 3));
    rt.registry_mut().register_with(|id| Spinner::new(id, 3));
    rt.assign_unit_space(UnitId::new(1), S1).unwrap();

    let mut order = Vec::new();
    while let Ok(s) = rt.step() {
        order.push(s.unit.raw());
        rt.commit_step(&s.result, &s.effects).unwrap();
    }
    assert_eq!(order, vec![0, 1, 0, 1, 0, 1]);
}

#[test]
fn sync_state_hash_folds_child_space_reservations() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    let pre = rt.sync_state_hash();
    rt.space_reservations_mut(S1)
        .unwrap()
        .insert_or_replace(UnitId::new(0), ReservedLine::containing(0));
    assert_ne!(pre, rt.sync_state_hash());
    rt.space_reservations_mut(S1)
        .unwrap()
        .remove_if_present(UnitId::new(0));
    // Empty tables gate out entirely, restoring the pre-entry hash.
    assert_eq!(pre, rt.sync_state_hash());
}

#[test]
fn a_write_through_one_same_space_view_reaches_the_other() {
    // The kernel admits mapping one shared segment at two addresses;
    // both views name the same bytes, so a store through one must be
    // readable through the other and must clear reservations held on
    // the alias line.
    let mut rt = build(16);
    rt.register_shared_mapping(
        3,
        0x40,
        &[
            (AddressSpaceId::BOOT, 0x2000),
            (AddressSpaceId::BOOT, 0x5000),
        ],
    )
    .unwrap();
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0x2010, 0x9D));
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(9), ReservedLine::containing(0x5010));

    let s = rt.step().unwrap();
    let outcome = rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(read4(rt.memory(), 0x5010).unwrap(), vec![0x9D; 4]);
    assert!(!rt.reservations().is_held_by(UnitId::new(9)));
    assert_eq!(outcome.reservations_cleared, 1);
}

#[test]
fn shared_alias_ranges_include_same_space_views() {
    let mut rt = build(16);
    rt.register_shared_mapping(
        4,
        0x40,
        &[
            (AddressSpaceId::BOOT, 0x2000),
            (AddressSpaceId::BOOT, 0x5000),
        ],
    )
    .unwrap();
    let range = ByteRange::new(GuestAddr::new(0x2010), 4).unwrap();
    let aliases = rt.shared_alias_ranges(UnitId::new(0), range);
    assert_eq!(
        aliases.len(),
        1,
        "the same-space sibling view aliases the range"
    );
    assert_eq!(aliases[0].start().raw(), 0x5010);
    assert_eq!(aliases[0].length(), 4);
}

#[test]
fn a_rejected_view_leaves_no_partial_regions_behind() {
    let mut rt = build(16);
    rt.create_address_space(S1).unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .install_region(0x3000, 0x40, "child", PageSize::Page64K)
        .unwrap();
    // Second view collides with the pre-existing child region; the
    // first view must not survive the failed registration.
    let err = rt
        .register_shared_mapping(5, 0x40, &[(AddressSpaceId::BOOT, 0x2000), (S1, 0x3000)])
        .unwrap_err();
    assert!(matches!(err, SpaceError::ViewInstall(_)));
    assert!(
        read4(rt.memory(), 0x2000).is_none(),
        "the boot view must not be installed by a failed registration",
    );
    // The key and the boot range are both free: a corrected
    // registration under the same key succeeds.
    rt.register_shared_mapping(5, 0x40, &[(AddressSpaceId::BOOT, 0x2000), (S1, 0x4000)])
        .unwrap();
}

#[test]
fn overlapping_views_in_one_registration_are_rejected_atomically() {
    let mut rt = build(16);
    let err = rt
        .register_shared_mapping(
            6,
            0x40,
            &[
                (AddressSpaceId::BOOT, 0x2000),
                (AddressSpaceId::BOOT, 0x2020),
            ],
        )
        .unwrap_err();
    assert!(matches!(err, SpaceError::ViewInstall(_)));
    assert!(read4(rt.memory(), 0x2000).is_none());
}

#[test]
fn a_view_running_off_the_end_of_the_address_space_is_rejected() {
    let mut rt = build(16);
    let err = rt
        .register_shared_mapping(7, 0x40, &[(AddressSpaceId::BOOT, u64::MAX - 0x10)])
        .unwrap_err();
    assert!(matches!(err, SpaceError::ViewInstall(_)));
}

#[test]
fn two_process_run_is_deterministic() {
    let run = || {
        let mut rt = build(16);
        rt.create_address_space(S1).unwrap();
        rt.register_shared_mapping(11, 0x40, &[(AddressSpaceId::BOOT, 0x2000), (S1, 0x3000)])
            .unwrap();
        rt.registry_mut()
            .register_with(|id| AddrWriter::new(id, 0x2000, 0x11));
        rt.registry_mut()
            .register_with(|id| AddrWriter::new(id, 0x3004, 0x22));
        rt.assign_unit_space(UnitId::new(1), S1).unwrap();
        let mut hashes = Vec::new();
        while let Ok(s) = rt.step() {
            rt.commit_step(&s.result, &s.effects).unwrap();
            hashes.push((rt.sync_state_hash(), rt.committed_memory_hash()));
        }
        hashes
    };
    let a = run();
    let b = run();
    assert!(!a.is_empty());
    assert_eq!(a, b, "two-process schedule must be bit-identical");
}

/// The first view's pre-attach content seeds the attaching view, and
/// post-attach stores replicate both directions through the commit
/// pipeline's fanout.
#[test]
fn keyed_shm_views_cohere_across_address_spaces() {
    use cellgov_lv2::request::classify;
    const KEY: u64 = 0x8006_0100_0000_0010;
    const BOOT_VIEW: u64 = 0x3000_0000;
    const CHILD_VIEW: u64 = 0x3100_0000;
    const SIZE: u64 = 0x10000;
    const FLAGS_64K: u64 = cellgov_ps3_abi::sys_memory::page_size::FLAG_64K;

    let mut rt = build(0x1000);
    rt.create_address_space(S1).unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .install_region(0, 0x1000, "child", PageSize::Page64K)
        .unwrap();

    // Boot unit 0 and child-space unit 1; both idle until the
    // post-attach writers run.
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, BOOT_VIEW + 0x200, 0x5A));
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, CHILD_VIEW + 0x300, 0xC3));
    rt.assign_unit_space(UnitId::new(1), S1).unwrap();

    // Boot: keyed 332 + 334 at BOOT_VIEW.
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
            &[KEY, SIZE, FLAGS_64K, 0x20, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );
    let mem_id = u64::from(u32::from_be_bytes(
        read4(rt.memory(), 0x20).unwrap().try_into().unwrap(),
    ));
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_MAP_SHARED_MEMORY,
            &[BOOT_VIEW, mem_id, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );

    // Pre-attach content through the boot view.
    rt.memory_mut()
        .apply_commit(
            ByteRange::new(GuestAddr::new(BOOT_VIEW + 0x100), 4).unwrap(),
            &[0xAB; 4],
        )
        .unwrap();

    // Child: keyed 332 attach (same mem_id back) + 334 at CHILD_VIEW.
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
            &[KEY, SIZE, FLAGS_64K, 0x20, 0, 0, 0, 0],
        ),
        UnitId::new(1),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(1)),
        Some(0)
    );
    let child_mem_id = u64::from(u32::from_be_bytes(
        read4(rt.space_memory(S1).unwrap(), 0x20)
            .unwrap()
            .try_into()
            .unwrap(),
    ));
    assert_eq!(
        child_mem_id, mem_id,
        "keyed 332 must return the same handle"
    );
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_MAP_SHARED_MEMORY,
            &[CHILD_VIEW, mem_id, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(1),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(1)),
        Some(0)
    );

    // The attach observed the pre-attach content.
    assert_eq!(
        read4(rt.space_memory(S1).unwrap(), CHILD_VIEW + 0x100).unwrap(),
        vec![0xAB; 4],
        "the attaching view must be seeded with the segment's current bytes",
    );

    // Post-attach stores replicate both directions.
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    let s = rt.step().unwrap();
    rt.commit_step(&s.result, &s.effects).unwrap();
    assert_eq!(
        read4(rt.space_memory(S1).unwrap(), CHILD_VIEW + 0x200).unwrap(),
        vec![0x5A; 4],
        "a boot-view store must appear through the child view",
    );
    assert_eq!(
        read4(rt.memory(), BOOT_VIEW + 0x300).unwrap(),
        vec![0xC3; 4],
        "a child-view store must appear through the boot view",
    );
}

/// Boundary case: keyed maps confined to one address space never
/// register a shared mapping, so single-process boots keep their
/// hash channels untouched.
#[test]
fn single_space_keyed_maps_do_not_register_a_mapping() {
    use cellgov_lv2::request::classify;
    let mut rt = build(0x1000);
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0x40, 0));
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
            &[
                0x8006_0100_0000_0020,
                0x10000,
                cellgov_ps3_abi::sys_memory::page_size::FLAG_64K,
                0x20,
                0,
                0,
                0,
                0,
            ],
        ),
        UnitId::new(0),
    );
    let mem_id = u64::from(u32::from_be_bytes(
        read4(rt.memory(), 0x20).unwrap().try_into().unwrap(),
    ));
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_MAP_SHARED_MEMORY,
            &[0x3000_0000, mem_id, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );
    assert!(
        rt.shared_alias_ranges(
            UnitId::new(0),
            ByteRange::new(GuestAddr::new(0x3000_0010), 4).unwrap()
        )
        .is_empty(),
        "a single-space keyed map must not promote into the shared table",
    );
}

/// The 334 occupancy check reads the CALLER's space. A window
/// occupied in the child's layout but free in the boot layout is
/// EBUSY for the child and OK for boot.
#[test]
fn map_occupancy_is_judged_in_the_callers_space() {
    use cellgov_lv2::request::classify;
    let mut rt = build(0x1000);
    rt.create_address_space(S1).unwrap();
    let child_mem = rt.space_memory_mut(S1).unwrap();
    child_mem
        .install_region(0, 0x1000, "child", PageSize::Page64K)
        .unwrap();
    // Child image occupies the window the guest will ask for.
    child_mem
        .install_region(0x5000_0000, 0x10000, "child_image", PageSize::Page64K)
        .unwrap();
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0x40, 0));
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0x40, 0));
    rt.assign_unit_space(UnitId::new(1), S1).unwrap();

    // Keyless 332 from boot mints the handle both callers name.
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
            &[
                0,
                0x10000,
                cellgov_ps3_abi::sys_memory::page_size::FLAG_64K,
                0x20,
                0,
                0,
                0,
                0,
            ],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );
    let mem_id = u64::from(u32::from_be_bytes(
        read4(rt.memory(), 0x20).unwrap().try_into().unwrap(),
    ));

    // Child: EBUSY, its own layout occupies the window.
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_MAP_SHARED_MEMORY,
            &[0x5000_0000, mem_id, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(1),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(1)),
        Some(cellgov_ps3_abi::cell_errors::CELL_EBUSY.into()),
        "the child's occupied window must refuse with EBUSY",
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("dispatch.mmapper_region_install_overlap"),
        0,
        "the refusal happens at dispatch, before any install is staged",
    );

    // Boot: same window, free in ITS layout -- the map succeeds.
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_MAP_SHARED_MEMORY,
            &[0x5000_0000, mem_id, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );
    assert!(
        read4(rt.memory(), 0x5000_0000).is_some(),
        "boot's map must install its window",
    );
}

/// The kernel binds one backing store to every address a shared
/// segment is mapped at (RPCS3 sys_mmapper.cpp
/// sys_mmapper_map_shared_memory), so a repeat map inside the first
/// space must not keep its own zero-filled bytes once the segment
/// becomes shared.
#[test]
fn a_repeat_map_in_one_space_is_seeded_when_the_segment_becomes_shared() {
    use cellgov_lv2::request::classify;
    const KEY: u64 = 0x8006_0100_0000_0030;
    const VIEW_A: u64 = 0x3000_0000;
    const VIEW_B: u64 = 0x3100_0000;
    const CHILD_VIEW: u64 = 0x3200_0000;
    const SIZE: u64 = 0x10000;
    const FLAGS_64K: u64 = cellgov_ps3_abi::sys_memory::page_size::FLAG_64K;

    let mut rt = build(0x1000);
    rt.create_address_space(S1).unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .install_region(0, 0x1000, "child", PageSize::Page64K)
        .unwrap();
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0x40, 0));
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0x40, 0));
    rt.assign_unit_space(UnitId::new(1), S1).unwrap();

    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
            &[KEY, SIZE, FLAGS_64K, 0x20, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );
    let mem_id = u64::from(u32::from_be_bytes(
        read4(rt.memory(), 0x20).unwrap().try_into().unwrap(),
    ));

    for view in [VIEW_A, VIEW_B] {
        rt.dispatch_lv2_request(
            classify(
                cellgov_ps3_abi::syscall::MMAPPER_MAP_SHARED_MEMORY,
                &[view, mem_id, 0, 0, 0, 0, 0, 0],
            ),
            UnitId::new(0),
        );
        assert_eq!(
            rt.registry_mut().drain_syscall_return(UnitId::new(0)),
            Some(0),
            "the kernel admits a repeat map of one segment",
        );
    }

    // Content written through the first view only; the second view is
    // still an independent zero-filled region at this point.
    rt.memory_mut()
        .apply_commit(
            ByteRange::new(GuestAddr::new(VIEW_A + 0x100), 4).unwrap(),
            &[0xAB; 4],
        )
        .unwrap();

    // A second address space attaches, promoting the segment.
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
            &[KEY, SIZE, FLAGS_64K, 0x20, 0, 0, 0, 0],
        ),
        UnitId::new(1),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(1)),
        Some(0)
    );
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_MAP_SHARED_MEMORY,
            &[CHILD_VIEW, mem_id, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(1),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(1)),
        Some(0)
    );

    assert_eq!(
        read4(rt.memory(), VIEW_B + 0x100).unwrap(),
        vec![0xAB; 4],
        "the same-space repeat map must be seeded from the segment",
    );
    assert_eq!(
        read4(rt.space_memory(S1).unwrap(), CHILD_VIEW + 0x100).unwrap(),
        vec![0xAB; 4],
        "the attaching view must be seeded from the segment",
    );
}

#[test]
fn seeding_an_attaching_view_clears_that_spaces_reservations() {
    use cellgov_lv2::request::classify;
    const KEY: u64 = 0x8006_0100_0000_0040;
    const BOOT_VIEW: u64 = 0x3000_0000;
    const CHILD_VIEW: u64 = 0x3100_0000;
    const SIZE: u64 = 0x10000;
    const FLAGS_64K: u64 = cellgov_ps3_abi::sys_memory::page_size::FLAG_64K;

    let mut rt = build(0x1000);
    rt.create_address_space(S1).unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .install_region(0, 0x1000, "child", PageSize::Page64K)
        .unwrap();
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0x40, 0));
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0x40, 0));
    rt.assign_unit_space(UnitId::new(1), S1).unwrap();

    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
            &[KEY, SIZE, FLAGS_64K, 0x20, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    let mem_id = u64::from(u32::from_be_bytes(
        read4(rt.memory(), 0x20).unwrap().try_into().unwrap(),
    ));
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_MAP_SHARED_MEMORY,
            &[BOOT_VIEW, mem_id, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );
    rt.memory_mut()
        .apply_commit(
            ByteRange::new(GuestAddr::new(BOOT_VIEW + 0x100), 4).unwrap(),
            &[0xAB; 4],
        )
        .unwrap();

    // A holder in the attaching space over bytes the seed rewrites,
    // plus one outside the segment that must survive.
    rt.space_reservations_mut(S1)
        .unwrap()
        .insert_or_replace(UnitId::new(9), ReservedLine::containing(CHILD_VIEW + 0x100));
    rt.space_reservations_mut(S1)
        .unwrap()
        .insert_or_replace(UnitId::new(8), ReservedLine::containing(0x40));

    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
            &[KEY, SIZE, FLAGS_64K, 0x20, 0, 0, 0, 0],
        ),
        UnitId::new(1),
    );
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_MAP_SHARED_MEMORY,
            &[CHILD_VIEW, mem_id, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(1),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(1)),
        Some(0)
    );

    assert!(!rt
        .space_reservations(S1)
        .unwrap()
        .is_held_by(UnitId::new(9)));
    assert!(rt
        .space_reservations(S1)
        .unwrap()
        .is_held_by(UnitId::new(8)));
}

/// A view whose length is not the live mapping's length is not a view
/// of that segment: appending it would make every later offset
/// translation in the fanout resolve against the wrong size.
#[test]
fn a_keyed_view_of_a_different_size_is_refused_against_a_live_mapping() {
    use cellgov_lv2::request::classify;
    const KEY: u64 = 0x8006_0100_0000_0050;
    const FLAGS_64K: u64 = cellgov_ps3_abi::sys_memory::page_size::FLAG_64K;

    let mut rt = build(0x1000);
    rt.create_address_space(S1).unwrap();
    rt.space_memory_mut(S1)
        .unwrap()
        .install_region(0, 0x1000, "child", PageSize::Page64K)
        .unwrap();
    rt.register_shared_mapping(KEY, 0x40, &[(AddressSpaceId::BOOT, 0x2000), (S1, 0x3000)])
        .unwrap();
    rt.registry_mut()
        .register_with(|id| AddrWriter::new(id, 0x40, 0));

    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
            &[KEY, 0x10000, FLAGS_64K, 0x20, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    let mem_id = u64::from(u32::from_be_bytes(
        read4(rt.memory(), 0x20).unwrap().try_into().unwrap(),
    ));
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::MMAPPER_MAP_SHARED_MEMORY,
            &[0x4000_0000, mem_id, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );

    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("spaces.keyed_shm_size_drift"),
        1,
        "a size mismatch against a live mapping must be named",
    );
    assert!(
        rt.shared_alias_ranges(
            UnitId::new(0),
            ByteRange::new(GuestAddr::new(0x4000_0000), 4).unwrap()
        )
        .is_empty(),
        "the mismatched view must not join the mapping",
    );
    // The pre-existing views are untouched.
    assert_eq!(
        rt.shared_alias_ranges(
            UnitId::new(0),
            ByteRange::new(GuestAddr::new(0x2000), 4).unwrap()
        )
        .len(),
        1,
    );
}
