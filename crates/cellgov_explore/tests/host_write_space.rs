//! Whether a host write resolves against the space it landed in.
//!
//! Equal numeric addresses in two spaces are different memory, and a
//! shared mapping is how one span of bytes shows up in both. The
//! runtime lands some host writes outside the space of the unit whose
//! step they belong to: a wake continuation lands in the waiter's
//! space, and a timer expiry's repair write in the expiring waiter's
//! space. Widening one of those through the stepping unit's space
//! looks for the range in the wrong views and finds none.
//!
//! A unit's own access to the sibling view still meets such a write:
//! `expand_aliases` widens that access from the unit's own space, and
//! the two meet on the raw range. Two host writes in two spaces have no
//! second chance, because `note_host_writes` is the one producer
//! `expand_aliases` no longer reaches. They prune, and that is a false
//! independence. So [`StepFootprint::note_host_writes`] widens each
//! entry through the space the record carries.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_explore::dependency::StepFootprint;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

const CHILD: AddressSpaceId = AddressSpaceId::new(1);
const BOOT_VIEW: u64 = 0x2000;
const CHILD_VIEW: u64 = 0x6000;
const VIEW_SIZE: u64 = 0x40;
const MAPPING_ID: u64 = 4;

/// One span of bytes mapped into both spaces, at different addresses.
fn two_spaces_sharing_a_mapping() -> Runtime {
    // The boot region stops well below both view bases, so neither
    // view overlaps it.
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(4), 100);
    rt.create_address_space(CHILD).unwrap();
    // `register_shared_mapping` installs each view's region itself.
    rt.register_shared_mapping(
        MAPPING_ID,
        VIEW_SIZE,
        &[(AddressSpaceId::BOOT, BOOT_VIEW), (CHILD, CHILD_VIEW)],
    )
    .unwrap();
    rt
}

fn child_range() -> ByteRange {
    ByteRange::new(GuestAddr::new(CHILD_VIEW), 8).unwrap()
}

/// A child-space write widened through a boot-space unit finds no
/// alias, so nothing pairs it with a second host write into the boot
/// view.
#[test]
fn the_alias_of_a_child_space_range_is_found_only_from_that_space() {
    let rt = two_spaces_sharing_a_mapping();

    let from_child = rt.shared_alias_ranges_in(CHILD, child_range());
    assert_eq!(
        from_child
            .iter()
            .map(|r| r.start().raw())
            .collect::<Vec<_>>(),
        vec![BOOT_VIEW],
        "the child view's bytes show up at the boot view's address",
    );

    assert!(
        rt.shared_alias_ranges_in(AddressSpaceId::BOOT, child_range())
            .is_empty(),
        "the same numeric range lies in no boot view, so asking from the \
         boot space finds nothing -- which is what widening a child-space \
         write through a boot-space unit does",
    );
}

/// The space-keyed form did not replace the question `expand_aliases`
/// asks.
#[test]
fn a_units_own_range_still_widens_through_the_units_space() {
    let mut rt = two_spaces_sharing_a_mapping();
    let boot_unit = rt.register_unit_with(|id| cellgov_testkit::world::CountingUnit::new(id, 1));

    let mut footprint = StepFootprint {
        shared_writes: vec![ByteRange::new(GuestAddr::new(BOOT_VIEW), 8).unwrap()],
        ..StepFootprint::default()
    };
    footprint.expand_aliases(&rt, boot_unit);

    let starts: Vec<u64> = footprint
        .shared_writes
        .iter()
        .map(|r| r.start().raw())
        .collect();
    assert!(
        starts.contains(&CHILD_VIEW),
        "the boot unit writes the shared bytes, so the child view aliases \
         them: {starts:?}",
    );
}

/// Both sides are built by hand, so this is the mechanical half: one
/// widened side is enough for the pair to meet. In a run the reader
/// would be a unit, and `expand_aliases` would widen it from its own
/// space; two host writes are the pair with no second chance.
#[test]
fn one_span_through_two_views_conflicts() {
    let rt = two_spaces_sharing_a_mapping();

    let mut writer = StepFootprint {
        shared_writes: vec![child_range()],
        ..StepFootprint::default()
    };
    writer
        .shared_writes
        .extend(rt.shared_alias_ranges_in(CHILD, child_range()));

    let reader = StepFootprint {
        shared_reads: vec![ByteRange::new(GuestAddr::new(BOOT_VIEW), 8).unwrap()],
        ..StepFootprint::default()
    };

    assert!(
        writer.conflicts(&reader),
        "the two name one span of bytes through two views",
    );
    assert!(
        !StepFootprint {
            shared_writes: vec![child_range()],
            ..StepFootprint::default()
        }
        .conflicts(&reader),
        "and without the alias they prune, which is the false independence",
    );
}

const SEC_OUT: u64 = 0x100;
const NSEC_OUT: u64 = 0x108;

/// Calls `sys_time_get_current_time` once, then finishes.
///
/// It emits no effect of its own, so the out parameters reach the
/// footprint through the published host writes or not at all.
#[derive(Clone)]
struct TimeCaller {
    id: cellgov_event::UnitId,
    steps: std::cell::Cell<u64>,
}

impl cellgov_exec::ExecutionUnit for TimeCaller {
    type Snapshot = u64;

    fn unit_id(&self) -> cellgov_event::UnitId {
        self.id
    }

    fn status(&self) -> cellgov_exec::UnitStatus {
        if self.steps.get() >= 1 {
            cellgov_exec::UnitStatus::Finished
        } else {
            cellgov_exec::UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &cellgov_exec::ExecutionContext<'_>,
        _effects: &mut Vec<cellgov_effects::Effect>,
    ) -> cellgov_exec::ExecutionStepResult {
        self.steps.set(self.steps.get() + 1);
        let mut args = [0u64; 9];
        args[0] = cellgov_ps3_abi::lv2::syscall::TIME_GET_CURRENT_TIME;
        args[1] = SEC_OUT;
        args[2] = NSEC_OUT;
        cellgov_exec::ExecutionStepResult {
            yield_reason: cellgov_exec::YieldReason::Syscall,
            consumed_cost: cellgov_time::InstructionCost::new(budget.raw()),
            local_diagnostics: cellgov_exec::LocalDiagnostics::with_pc(0x1000),
            fault: None,
            syscall_args: Some(args),
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

/// The other tests read `Runtime::shared_alias_ranges_in` directly.
/// This one holds the consumer to it: the step names no write of its
/// own, so the out parameter reaches `shared_writes` through
/// `note_host_writes` or not at all.
#[test]
fn a_published_host_write_reaches_the_footprint_with_its_space() {
    let mut rt = two_spaces_sharing_a_mapping();
    rt.register_unit_with(|id| TimeCaller {
        id,
        steps: std::cell::Cell::new(0),
    });

    let step = rt.step().expect("the caller is runnable");
    assert!(
        step.effects.is_empty(),
        "the syscall step names no write of its own: {:?}",
        step.effects,
    );
    rt.commit_step(&step.result, &step.effects)
        .expect("the dispatch commits");

    let published = rt.last_host_writes().to_vec();
    let out_param = published
        .iter()
        .find(|(_, _, range)| range.start().raw() == NSEC_OUT)
        .map(|(writer, space, _)| (*writer, *space));
    assert_eq!(
        out_param,
        Some((cellgov_trace::HostWriter::Lv2Effect, AddressSpaceId::BOOT)),
        "the handler's write is published under a writer `note_host_writes` \
         records rather than skips, against the space it landed in: {published:?}",
    );

    let mut footprint = StepFootprint::default();
    footprint.note_host_writes(&rt);
    assert!(
        footprint
            .shared_writes
            .iter()
            .any(|r| r.start().raw() == NSEC_OUT),
        "and the consumer takes the range from that record: {:?}",
        footprint.shared_writes,
    );
    assert!(
        footprint
            .shared_writes
            .iter()
            .all(|r| r.start().raw() != CHILD_VIEW),
        "the bytes lie in no shared view, so nothing widens them: {:?}",
        footprint.shared_writes,
    );
}
