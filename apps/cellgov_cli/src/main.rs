//! cellgov_cli -- run scenarios, dump traces, inspect scheduler decisions.
//!
//! Commands:
//!
//! - `cellgov_cli <scenario>` -- run a scenario, print deterministic summary.
//! - `cellgov_cli dump <scenario>` -- run a scenario, print every trace record.
//!
//! Available scenarios: fairness, conflict, mailbox, dma, send, signal, isa.

use cellgov_testkit::fixtures;
use cellgov_testkit::runner::{run, ScenarioOutcome, ScenarioResult};
use cellgov_trace::TraceReader;

/// Supported scenario names and the fixture factories that produce them.
fn run_scenario(name: &str) -> Option<(&str, ScenarioResult)> {
    let (label, fixture) = match name {
        "fairness" => (
            "round-robin-fairness(3 units, 5 steps each)",
            fixtures::round_robin_fairness_scenario(3, 5),
        ),
        "conflict" => (
            "write-conflict(3 steps each)",
            fixtures::write_conflict_scenario(3),
        ),
        "mailbox" => (
            "mailbox-roundtrip(command=0x42)",
            fixtures::mailbox_roundtrip_scenario(0x42),
        ),
        "dma" => ("dma-block-unblock", fixtures::dma_block_unblock_scenario()),
        "send" => (
            "mailbox-send(5 messages)",
            fixtures::mailbox_send_scenario(5),
        ),
        "signal" => ("signal-update(4 bits)", fixtures::signal_update_scenario(4)),
        "isa" => ("fake-isa-integration", fixtures::fake_isa_scenario()),
        _ => return None,
    };
    Some((label, run(fixture)))
}

const SCENARIOS: &[&str] = &[
    "fairness", "conflict", "mailbox", "dma", "send", "signal", "isa",
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        println!("usage: cellgov_cli <scenario>");
        println!("       cellgov_cli dump <scenario>");
        println!();
        println!("available scenarios:");
        for name in SCENARIOS {
            println!("  {name}");
        }
        std::process::exit(0);
    }

    if args[1] == "dump" {
        let name = args.get(2).map(String::as_str).unwrap_or_else(|| {
            eprintln!("usage: cellgov_cli dump <scenario>");
            std::process::exit(1);
        });
        match run_scenario(name) {
            Some((_label, result)) => dump_trace(&result),
            None => {
                eprintln!("unknown scenario: {name}");
                eprintln!("available: {}", SCENARIOS.join(", "));
                std::process::exit(1);
            }
        }
        return;
    }

    let name = &args[1];
    match run_scenario(name) {
        Some((label, result)) => println!("{}", report(label, &result)),
        None => {
            eprintln!("unknown scenario: {name}");
            eprintln!("available: {}", SCENARIOS.join(", "));
            std::process::exit(1);
        }
    }
}

/// Print every trace record from a scenario run, one per line.
fn dump_trace(result: &ScenarioResult) {
    use cellgov_trace::{TraceRecord, TracedBlockReason, TracedWakeReason};

    for (i, rec) in TraceReader::new(&result.trace_bytes)
        .map(|r| r.expect("trace decode failed"))
        .enumerate()
    {
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
        }
    }
    let count = TraceReader::new(&result.trace_bytes).count();
    println!("--- {count} records total ---");
}

/// Format a [`ScenarioResult`] as a deterministic, ASCII-only summary.
fn report(name: &str, result: &ScenarioResult) -> String {
    let outcome = match result.outcome {
        ScenarioOutcome::Stalled => "Stalled",
        ScenarioOutcome::MaxStepsExceeded => "MaxStepsExceeded",
    };
    format!(
        "scenario: {name}\noutcome: {outcome}\nsteps_taken: {steps}\ntrace_bytes: {bytes}\nmemory_hash: 0x{mem:016x}\nstatus_hash: 0x{status:016x}\nsync_hash: 0x{sync:016x}",
        steps = result.steps_taken,
        bytes = result.trace_bytes.len(),
        mem = result.final_memory_hash.raw(),
        status = result.final_unit_status_hash.raw(),
        sync = result.final_sync_hash.raw(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_scenario_runs_successfully() {
        for name in SCENARIOS {
            let (label, result) =
                run_scenario(name).unwrap_or_else(|| panic!("scenario {name} not found"));
            assert_eq!(
                result.outcome,
                ScenarioOutcome::Stalled,
                "scenario {label} did not stall cleanly"
            );
            assert!(result.steps_taken > 0, "scenario {label} took zero steps");
        }
    }

    #[test]
    fn unknown_scenario_returns_none() {
        assert!(run_scenario("nonexistent").is_none());
    }

    #[test]
    fn report_is_deterministic_across_runs() {
        let (l1, r1) = run_scenario("fairness").unwrap();
        let (l2, r2) = run_scenario("fairness").unwrap();
        assert_eq!(report(l1, &r1), report(l2, &r2));
    }

    #[test]
    fn dump_does_not_panic_for_any_scenario() {
        for name in SCENARIOS {
            let (_, result) =
                run_scenario(name).unwrap_or_else(|| panic!("scenario {name} not found"));
            // Just verify decoding succeeds for every record.
            let records: Vec<_> = TraceReader::new(&result.trace_bytes)
                .map(|r| r.expect("decode"))
                .collect();
            assert!(
                !records.is_empty(),
                "scenario {name} produced no trace records"
            );
        }
    }

    #[test]
    fn report_includes_sync_hash_field() {
        let (label, result) = run_scenario("isa").unwrap();
        let r = report(label, &result);
        assert!(r.contains("sync_hash: 0x"));
    }
}
