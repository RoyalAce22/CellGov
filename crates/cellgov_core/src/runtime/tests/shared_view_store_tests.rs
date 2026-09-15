//! A store through one view spares its own reservation over the others.
//!
//! Every view of a shared segment names one reservation granule, so the
//! split the commit pipeline applies in the view a store lands in is
//! the split the replicated writes carry into the siblings: the storing
//! unit keeps its reservation, every other holder loses theirs.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_sync::ReservedLine;
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use crate::commit::CommitOutcome;
use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::state::Runtime;

const S1: AddressSpaceId = AddressSpaceId::new(1);

const KEY: u64 = 7;
const SEG_SIZE: u64 = 0x40;

/// Three views of one segment: two in space 0, one in the child. The
/// repeat map is what puts the storing unit's own reservation over an
/// alias of the bytes it writes, without a second process.
const VIEW0_BASE: u64 = 0x2000;
const ALIAS_BASE: u64 = 0x2100;
const VIEW1_BASE: u64 = 0x3000;

/// Where in the segment the store lands.
const OFFSET: u64 = 0x10;

const LEN: u64 = 4;
const MARK: u8 = 0xC7;

/// The storing unit, and two other holders.
const WRITER: UnitId = UnitId::new(0);
const OTHER_SAME_SPACE: UnitId = UnitId::new(8);
const OTHER_CHILD_SPACE: UnitId = UnitId::new(9);

fn range(addr: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), LEN).expect("a 4-byte range")
}

/// Writes [`MARK`] once through view 0 and finishes.
#[derive(Clone)]
struct ViewWriter {
    id: UnitId,
    done: bool,
}

impl ExecutionUnit for ViewWriter {
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
        effects.push(Effect::shared_write(
            range(VIEW0_BASE + OFFSET),
            WritePayload::new(vec![MARK; LEN as usize]),
            self.id,
            GuestTicks::ZERO,
        ));
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

/// One segment in three views, with the writer registered.
fn segment_with_writer() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(4), 100);
    rt.create_address_space(S1).expect("a fresh space id");
    rt.register_shared_mapping(
        KEY,
        SEG_SIZE,
        &[
            (AddressSpaceId::BOOT, VIEW0_BASE),
            (AddressSpaceId::BOOT, ALIAS_BASE),
            (S1, VIEW1_BASE),
        ],
    )
    .expect("three non-overlapping views of one fresh key");
    let id = rt
        .registry_mut()
        .register_with(|id| ViewWriter { id, done: false });
    assert_eq!(id, WRITER, "registration order moved the writer");
    rt
}

fn commit_the_store(rt: &mut Runtime) -> CommitOutcome {
    let step = rt.step().expect("the writer runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the view is mapped and writable")
}

/// The premise: the store reaches all three views, so a reservation
/// over any of them covers bytes the store replaced.
#[test]
fn the_store_reaches_every_view_of_the_segment() {
    let mut rt = segment_with_writer();
    commit_the_store(&mut rt);

    let want = Some(vec![MARK; LEN as usize]);
    assert_eq!(
        rt.memory()
            .read(range(VIEW0_BASE + OFFSET))
            .map(<[u8]>::to_vec),
        want,
        "the view the store landed in",
    );
    assert_eq!(
        rt.memory()
            .read(range(ALIAS_BASE + OFFSET))
            .map(<[u8]>::to_vec),
        want,
        "the repeat map in the same space",
    );
    let child = rt.space_memory(S1).expect("the child space exists");
    assert_eq!(
        child.read(range(VIEW1_BASE + OFFSET)).map(<[u8]>::to_vec),
        want,
        "and the view in the child space",
    );
}

/// The storing unit keeps its reservation over the alias it did not
/// store through.
#[test]
fn a_store_spares_its_own_reservation_over_a_sibling_view() {
    let mut rt = segment_with_writer();
    rt.reservations_mut()
        .insert_or_replace(WRITER, ReservedLine::containing(ALIAS_BASE + OFFSET));

    let outcome = commit_the_store(&mut rt);

    assert_eq!(
        rt.reservations().get(WRITER),
        Some(ReservedLine::containing(ALIAS_BASE + OFFSET)),
        "one granule backs every view, and a unit's own store does not \
         clear its own reservation over it",
    );
    assert_eq!(
        outcome.reservations_cleared, 0,
        "the only holder was the storing unit, so nothing was swept",
    );
}

/// Every other holder loses theirs, in both spaces.
///
/// Without this the case above could pass against a replication that
/// swept nothing at all.
#[test]
fn a_store_sweeps_every_other_holder_over_every_view() {
    let mut rt = segment_with_writer();
    rt.reservations_mut().insert_or_replace(
        OTHER_SAME_SPACE,
        ReservedLine::containing(ALIAS_BASE + OFFSET),
    );
    rt.space_reservations_mut(S1)
        .expect("the child space has a table")
        .insert_or_replace(
            OTHER_CHILD_SPACE,
            ReservedLine::containing(VIEW1_BASE + OFFSET),
        );

    let outcome = commit_the_store(&mut rt);

    assert!(
        rt.reservations().get(OTHER_SAME_SPACE).is_none(),
        "another holder over the same-space alias loses its reservation",
    );
    assert!(
        rt.space_reservations(S1)
            .expect("the table survives")
            .get(OTHER_CHILD_SPACE)
            .is_none(),
        "and so does one over the child space's view",
    );
    // The fanout's clears reach the outcome through `commit_step`; a
    // count short of two would mean a sweep the witness never saw.
    assert_eq!(
        outcome.reservations_cleared, 2,
        "both holders are counted, one per view's space",
    );
}
