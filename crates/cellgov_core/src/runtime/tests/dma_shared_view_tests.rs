//! A DMA landing inside a shared view reaches the sibling views.
//!
//! The transfer writes space 0, and a shared segment's views name those
//! same bytes from other spaces. A landing that replicated nothing
//! would leave a sibling view holding what the transfer replaced, with
//! no record that a replication was owed.

use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_sync::ReservedLine;
use cellgov_time::{Budget, InstructionCost};

use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::state::Runtime;

const S1: AddressSpaceId = AddressSpaceId::new(1);

/// The shared segment's key, size, and the base each view maps it at.
const KEY: u64 = 7;
const SEG_SIZE: u64 = 0x40;
const VIEW0_BASE: u64 = 0x2000;
const VIEW1_BASE: u64 = 0x3000;

/// A repeat map of the segment inside space 0, far enough from
/// [`VIEW0_BASE`] that the two sit on different reservation lines.
const ALIAS_BASE: u64 = 0x2100;

/// The transfer's source, outside the segment, and its destination,
/// `OFFSET` bytes into view 0.
const SRC: u64 = 0x20;
const OFFSET: u64 = 0x10;

const LEN: u64 = 4;

/// The bytes the transfer moves, so each view says whether it saw them.
const PAYLOAD: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];

fn range(addr: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), LEN).expect("a 4-byte range")
}

/// Enqueues one payload-less put into the shared view, parks, and
/// finishes once the completion wakes it.
#[derive(Clone)]
struct DmaEmitter {
    id: UnitId,
    steps: u64,
}

impl ExecutionUnit for DmaEmitter {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps >= 2 {
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
        self.steps += 1;
        let yield_reason = if self.steps == 1 {
            let request = DmaRequest::new(
                DmaDirection::Put,
                range(SRC),
                range(VIEW0_BASE + OFFSET),
                self.id,
            )
            .expect("equal-length ends");
            effects.push(Effect::DmaEnqueue {
                request,
                payload: None,
            });
            YieldReason::DmaWait
        } else {
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

/// A segment mapped into space 0 and one child space, with a transfer
/// aimed at the space-0 view.
fn shared_segment_with_emitter() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(4), 100);
    rt.create_address_space(S1).expect("a fresh space id");
    rt.register_shared_mapping(
        KEY,
        SEG_SIZE,
        &[(AddressSpaceId::BOOT, VIEW0_BASE), (S1, VIEW1_BASE)],
    )
    .expect("two views of one fresh key");
    rt.registry_mut()
        .register_with(|id| DmaEmitter { id, steps: 0 });
    rt.place_bytes(AddressSpaceId::BOOT, range(SRC), &PAYLOAD)
        .expect("space 0 maps the source");
    rt
}

/// The same segment, mapped a second time inside space 0.
///
/// A unit belongs to one space, so a repeat map in the issuer's own
/// space is the only shape that puts the issuer's reservation over an
/// alias of the bytes its transfer writes.
fn shared_segment_with_same_space_alias() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(4), 100);
    rt.create_address_space(S1).expect("a fresh space id");
    rt.register_shared_mapping(
        KEY,
        SEG_SIZE,
        &[
            (AddressSpaceId::BOOT, VIEW0_BASE),
            (S1, VIEW1_BASE),
            (AddressSpaceId::BOOT, ALIAS_BASE),
        ],
    )
    .expect("three non-overlapping views of one fresh key");
    rt.registry_mut()
        .register_with(|id| DmaEmitter { id, steps: 0 });
    rt.place_bytes(AddressSpaceId::BOOT, range(SRC), &PAYLOAD)
        .expect("space 0 maps the source");
    rt
}

/// Runs until the completion has fired.
fn run_to_completion(rt: &mut Runtime) {
    for _ in 0..4 {
        let Ok(step) = rt.step() else { break };
        rt.commit_step(&step.result, &step.effects)
            .expect("no step of this workload refuses its commit");
    }
    assert!(
        rt.dma_queue().is_empty(),
        "the completion never came due, so the views say nothing",
    );
}

/// The premise: the two views start holding zero, and the segment
/// registration put a region in each space.
#[test]
fn both_views_start_zeroed() {
    let rt = shared_segment_with_emitter();
    assert_eq!(
        rt.memory()
            .read(range(VIEW0_BASE + OFFSET))
            .map(<[u8]>::to_vec),
        Some(vec![0u8; LEN as usize]),
        "the space-0 view is mapped and zeroed",
    );
    let child = rt.space_memory(S1).expect("the child space exists");
    assert_eq!(
        child.read(range(VIEW1_BASE + OFFSET)).map(<[u8]>::to_vec),
        Some(vec![0u8; LEN as usize]),
        "and so is the sibling view",
    );
}

/// The landing reaches the sibling view at its own base.
#[test]
fn a_dma_landing_in_a_view_replicates_to_its_sibling() {
    let mut rt = shared_segment_with_emitter();
    run_to_completion(&mut rt);

    assert_eq!(
        rt.memory()
            .read(range(VIEW0_BASE + OFFSET))
            .map(<[u8]>::to_vec),
        Some(PAYLOAD.to_vec()),
        "the transfer lands in the view it was aimed at",
    );
    let child = rt.space_memory(S1).expect("the child space exists");
    assert_eq!(
        child.read(range(VIEW1_BASE + OFFSET)).map(<[u8]>::to_vec),
        Some(PAYLOAD.to_vec()),
        "and the sibling view names the same bytes, at its own base",
    );
}

/// The replicated write reaches the sibling space's reservation table,
/// not only its memory.
///
/// `other` stands for a unit of that space holding the replicated
/// bytes. The issuer's exemption is the case below, because a space-0
/// unit cannot hold a reservation in a child space's table and this
/// fixture could only name that state by building it by hand.
#[test]
fn a_replicated_write_sweeps_the_sibling_space_table() {
    let mut rt = shared_segment_with_emitter();
    let other = UnitId::new(9);
    rt.space_reservations_mut(S1)
        .expect("the child space has a table")
        .insert_or_replace(other, ReservedLine::containing(VIEW1_BASE + OFFSET));

    run_to_completion(&mut rt);

    let table = rt.space_reservations(S1).expect("the table survives");
    assert!(
        table.get(other).is_none(),
        "a holder in the sibling space loses its reservation over the \
         replicated bytes",
    );
}

/// The issuer's exemption, over a repeat map inside its own space.
///
/// The issuer holds the reservation in the one table it can hold one
/// in, over an alias of the bytes its transfer writes.
#[test]
fn a_landing_spares_the_issuer_over_a_same_space_alias_and_sweeps_other_holders() {
    let mut rt = shared_segment_with_same_space_alias();
    let issuer = UnitId::new(0);
    let other = UnitId::new(9);
    rt.reservations_mut()
        .insert_or_replace(other, ReservedLine::containing(ALIAS_BASE + OFFSET));
    rt.reservations_mut()
        .insert_or_replace(issuer, ReservedLine::containing(ALIAS_BASE + OFFSET));

    run_to_completion(&mut rt);

    assert_eq!(
        rt.memory()
            .read(range(ALIAS_BASE + OFFSET))
            .map(<[u8]>::to_vec),
        Some(PAYLOAD.to_vec()),
        "a repeat map is an alias of the same bytes, so the landing reaches it",
    );
    assert!(
        rt.reservations().get(other).is_none(),
        "the alias names the granule the transfer wrote, so another \
         unit's reservation over it goes",
    );
    assert!(
        rt.reservations().get(issuer).is_some(),
        "and the issuer's own survives, as it does over the view the \
         transfer was aimed at",
    );
}
