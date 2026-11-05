//! A store that runs out of a shared view into the region beside it:
//! the part inside the view reaches every sibling view, and the alias
//! query names that part alone.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::state::Runtime;

const S1: AddressSpaceId = AddressSpaceId::new(1);
const KEY: u64 = 7;
const SEG_SIZE: u64 = 0x40;

/// Two views of one segment, and a plain region right after the first.
const VIEW0_BASE: u64 = 0x2000;
const AFTER_VIEW0: u64 = VIEW0_BASE + SEG_SIZE;
const VIEW1_BASE: u64 = 0x3000;

/// The store covers the last four bytes of view 0 and the first four of
/// the region after it.
const STORE_AT: u64 = AFTER_VIEW0 - 4;
const BYTES: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).expect("a valid range")
}

#[derive(Clone)]
struct SpanWriter {
    id: UnitId,
    done: bool,
}

impl ExecutionUnit for SpanWriter {
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
            range(STORE_AT, BYTES.len() as u64),
            WritePayload::new(BYTES.to_vec()),
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

fn runtime() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(4), 100);
    rt.create_address_space(S1).expect("a fresh space id");
    rt.register_shared_mapping(
        KEY,
        SEG_SIZE,
        &[(AddressSpaceId::BOOT, VIEW0_BASE), (S1, VIEW1_BASE)],
    )
    .expect("two non-overlapping views of one fresh key");
    rt.memory
        .install_region(AFTER_VIEW0, 0x40, "after_view", PageSize::Page4K)
        .expect("the range after view 0 is free");
    rt.registry_mut()
        .register_with(|id| SpanWriter { id, done: false });
    rt
}

#[test]
fn the_part_of_a_store_inside_a_view_reaches_the_sibling_view() {
    let mut rt = runtime();
    let mut step = rt.step().expect("the writer runs");
    rt.commit_step(&mut step)
        .expect("every byte lies in a writable region");
    assert_eq!(rt.memory().read(range(STORE_AT, 4)).unwrap(), &BYTES[..4]);
    assert_eq!(
        rt.memory().read(range(AFTER_VIEW0, 4)).unwrap(),
        &BYTES[4..]
    );
    let child = rt.space_memory(S1).expect("the child space exists");
    assert_eq!(
        child.read(range(VIEW1_BASE + SEG_SIZE - 4, 4)).unwrap(),
        &BYTES[..4],
        "the sibling view holds the part that landed in view 0",
    );
}

#[test]
fn the_alias_of_a_store_running_out_of_a_view_is_its_part_inside_the_view() {
    let rt = runtime();
    assert_eq!(
        rt.shared_alias_ranges_in(AddressSpaceId::BOOT, range(STORE_AT, 8)),
        vec![range(VIEW1_BASE + SEG_SIZE - 4, 4)],
    );
}

#[test]
fn the_alias_of_a_store_running_into_a_view_or_over_it_is_its_part_inside_the_view() {
    let rt = runtime();
    assert_eq!(
        rt.shared_alias_ranges_in(AddressSpaceId::BOOT, range(VIEW0_BASE - 4, 8)),
        vec![range(VIEW1_BASE, 4)],
        "a store that starts before the view aliases only its tail",
    );
    assert_eq!(
        rt.shared_alias_ranges_in(AddressSpaceId::BOOT, range(VIEW0_BASE - 4, SEG_SIZE + 8)),
        vec![range(VIEW1_BASE, SEG_SIZE)],
        "a store over the whole view aliases the whole sibling view",
    );
}

#[test]
fn a_write_from_a_view_whose_space_has_no_memory_is_recorded_and_not_replicated() {
    let mut rt = runtime();
    rt.spaces.extra.remove(&S1);
    let before = rt
        .memory()
        .read(range(VIEW0_BASE, SEG_SIZE))
        .unwrap()
        .to_vec();
    let cleared = rt.fanout_committed_range(S1, range(VIEW1_BASE, 4), None);
    assert_eq!(cleared, 0);
    assert_eq!(
        rt.memory().read(range(VIEW0_BASE, SEG_SIZE)).unwrap(),
        &before[..],
        "the sibling view in boot memory is unchanged",
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.fanout_source_space_missing"),
        1,
    );
}
