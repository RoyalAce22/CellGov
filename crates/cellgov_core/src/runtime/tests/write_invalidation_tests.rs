//! Every path that lands bytes in guest memory drops predecoded code
//! over them:
//!
//! - each host writer;
//! - a committed conditional store;
//! - an RSX label write.

use std::cell::RefCell;
use std::rc::Rc;

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks, InstructionCost};
use cellgov_trace::HostWriter;
use strum::VariantArray;

use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::state::Runtime;

/// The `(addr, len)` of every `invalidate_code` call the unit saw.
type Calls = Rc<RefCell<Vec<(u64, u64)>>>;

/// Emits `effects` on its one step, and records every
/// `invalidate_code` call it receives.
#[derive(Clone)]
struct Writer {
    id: UnitId,
    effects: Vec<Effect>,
    calls: Calls,
    done: bool,
}

impl ExecutionUnit for Writer {
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
        effects.append(&mut self.effects);
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

    fn caches_code(&self) -> bool {
        true
    }

    fn snapshot(&self) {}
}

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).expect("in-range test address")
}

/// A runtime over 0x3000 bytes of space 0 holding one [`Writer`] that
/// emits `effects`, and the writer's call log.
fn build(effects: Vec<Effect>) -> (Runtime, Calls) {
    let mut rt = Runtime::new(GuestMemory::new(0x3000), Budget::new(4), 100);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let unit_calls = Rc::clone(&calls);
    rt.registry_mut().register_with(move |id| Writer {
        id,
        effects,
        calls: unit_calls,
        done: false,
    });
    (rt, calls)
}

/// Runs the writer's one step and commits it.
fn step_and_commit(rt: &mut Runtime) {
    let mut step = rt.step().expect("the writer runs");
    rt.commit_step(&mut step).expect("the batch commits");
}

#[test]
fn every_host_writer_drops_predecoded_code_over_its_bytes() {
    assert_eq!(
        HostWriter::VARIANTS.len(),
        9,
        "a new writer joins this test"
    );
    for &writer in HostWriter::VARIANTS {
        let (mut rt, calls) = build(Vec::new());
        rt.host_write(
            writer,
            AddressSpaceId::BOOT,
            range(0x1010, 8),
            &[0xA5; 8],
            None,
        )
        .expect("space 0 maps the range");
        assert_eq!(*calls.borrow(), vec![(0x1010, 8)], "{writer:?}");
    }
}

#[test]
fn a_refused_host_write_drops_nothing() {
    let (mut rt, calls) = build(Vec::new());
    rt.host_write(
        HostWriter::Lv2Effect,
        AddressSpaceId::BOOT,
        range(0x8000, 8),
        &[0xA5; 8],
        None,
    )
    .expect_err("space 0 maps nothing at 0x8000");
    assert!(calls.borrow().is_empty(), "{:?}", calls.borrow());
}

#[test]
fn a_committed_conditional_store_drops_predecoded_code_over_its_bytes() {
    let effects = vec![Effect::ConditionalStore {
        range: range(0x1040, 4),
        bytes: WritePayload::new(vec![0x38, 0x60, 0x00, 0x01]),
        source: UnitId::new(0),
        source_time: GuestTicks::ZERO,
    }];
    let (mut rt, calls) = build(effects);
    step_and_commit(&mut rt);
    assert_eq!(*calls.borrow(), vec![(0x1040, 4)]);
}

#[test]
fn an_rsx_label_write_drops_predecoded_code_over_its_bytes() {
    let effects = vec![Effect::RsxLabelWrite {
        offset: 0x10,
        value: 0x3860_0001,
    }];
    let (mut rt, calls) = build(effects);
    rt.set_rsx_label_base(GuestAddr::new(0x2000));
    step_and_commit(&mut rt);
    assert_eq!(*calls.borrow(), vec![(0x2010, 4)]);
}
