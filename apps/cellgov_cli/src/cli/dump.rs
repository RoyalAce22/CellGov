//! `dump <scenario>` -- run a scenario and print every trace
//! record, one per line.

use cellgov_testkit::runner::ScenarioResult;
use cellgov_trace::TraceReader;

use super::exit::die;
use super::scenarios::run_scenario;

/// Entry point for `cellgov_cli dump <scenario>`.
pub(crate) fn run(args: &[String], scenarios_list: &[&str]) {
    let name = args
        .get(2)
        .map(String::as_str)
        .unwrap_or_else(|| die("usage: cellgov_cli dump <scenario>"));
    match run_scenario(name) {
        Some((_label, result)) => dump_trace(&result),
        None => die(&format!(
            "unknown scenario: {name}\navailable: {}",
            scenarios_list.join(", ")
        )),
    }
}

/// Print every trace record from a scenario run, one per line.
fn dump_trace(result: &ScenarioResult) {
    use cellgov_trace::{TraceRecord, TracedBlockReason, TracedWakeReason};

    // Track count in the single decoding pass. The old
    // implementation decoded the whole trace once for display and
    // then a second time just to count -- a real O(n) waste on
    // large traces. Single pass: `count = i + 1` on each iteration.
    let mut count = 0usize;
    for (i, rec) in TraceReader::new(&result.trace_bytes).enumerate() {
        let rec =
            rec.unwrap_or_else(|e| die(&format!("trace decode failed at record index {i}: {e:?}")));
        count = i + 1;
        match rec {
            TraceRecord::UnitScheduled {
                unit,
                granted_budget,
                time,
                epoch,
            } => {
                println!(
                    "{i:4}  UnitScheduled      unit={} budget={} time={} epoch={}",
                    unit.raw(),
                    granted_budget.raw(),
                    time.raw(),
                    epoch.raw()
                );
            }
            TraceRecord::StepCompleted {
                unit,
                yield_reason,
                consumed_budget,
                time_after,
            } => {
                println!(
                    "{i:4}  StepCompleted      unit={} yield={:?} consumed={} time_after={}",
                    unit.raw(),
                    yield_reason,
                    consumed_budget.raw(),
                    time_after.raw()
                );
            }
            TraceRecord::EffectEmitted {
                unit,
                sequence,
                kind,
            } => {
                println!(
                    "{i:4}  EffectEmitted      unit={} seq={} kind={:?}",
                    unit.raw(),
                    sequence,
                    kind
                );
            }
            TraceRecord::CommitApplied {
                unit,
                writes_committed,
                effects_deferred,
                fault_discarded,
                epoch_after,
            } => {
                println!(
                    "{i:4}  CommitApplied      unit={} writes={} deferred={} fault={} epoch_after={}",
                    unit.raw(),
                    writes_committed,
                    effects_deferred,
                    fault_discarded,
                    epoch_after.raw()
                );
            }
            TraceRecord::StateHashCheckpoint { kind, hash } => {
                println!(
                    "{i:4}  StateHashCheckpoint kind={:?} hash=0x{:016x}",
                    kind,
                    hash.raw()
                );
            }
            TraceRecord::UnitBlocked { unit, reason } => {
                let reason_str = match reason {
                    TracedBlockReason::WaitOnEvent => "WaitOnEvent",
                    TracedBlockReason::MailboxEmpty => "MailboxEmpty",
                };
                println!(
                    "{i:4}  UnitBlocked        unit={} reason={}",
                    unit.raw(),
                    reason_str
                );
            }
            TraceRecord::UnitWoken { unit, reason } => {
                let reason_str = match reason {
                    TracedWakeReason::WakeEffect => "WakeEffect",
                    TracedWakeReason::DmaCompletion => "DmaCompletion",
                };
                println!(
                    "{i:4}  UnitWoken          unit={} reason={}",
                    unit.raw(),
                    reason_str
                );
            }
            TraceRecord::PpuStateHash { step, pc, hash } => {
                println!(
                    "{i:4}  PpuStateHash       step={step} pc=0x{pc:x} hash=0x{:x}",
                    hash.raw()
                );
            }
            TraceRecord::PpuStateFull { step, pc, .. } => {
                println!("{i:4}  PpuStateFull       step={step} pc=0x{pc:x} (window capture)");
            }
        }
    }
    println!("--- {count} records total ---");
}
