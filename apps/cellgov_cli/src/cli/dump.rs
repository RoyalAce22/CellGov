//! `scenario dump <name>` -- run a scenario and print every trace
//! record.

use cellgov_testkit::runner::ScenarioResult;
use cellgov_trace::TraceReader;

use super::exit::CommandError;
use super::scenarios::run_scenario;

pub(crate) fn run(name: &str, scenarios_list: &[&str]) -> Result<(), CommandError> {
    match run_scenario(name) {
        Some((_label, result)) => dump_trace(&result),
        None => Err(CommandError::failed(format!(
            "unknown scenario: {name}\navailable: {}",
            scenarios_list.join(", ")
        ))),
    }
}

fn dump_trace(result: &ScenarioResult) -> Result<(), CommandError> {
    use cellgov_trace::{
        TraceRecord, TracedBlockReason, TracedInvariantBreakReason, TracedSyscallDisposition,
        TracedWakeReason,
    };

    let mut count = 0usize;
    for (i, rec) in TraceReader::new(&result.trace_bytes).enumerate() {
        let rec = rec.map_err(|error| {
            CommandError::failed(format!(
                "trace decode failed at record index {i}: {error:?}"
            ))
        })?;
        count = i + 1;
        match rec {
            TraceRecord::RunIdentity {
                format_version,
                firmware,
                game,
                overrides,
            } => {
                println!(
                    "{i:4}  RunIdentity        format_version={format_version} \
                     firmware=0x{firmware:016x} game=0x{game:016x} \
                     overrides=0x{overrides:016x}"
                );
            }
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
                consumed_cost,
                time_after,
            } => {
                println!(
                    "{i:4}  StepCompleted      unit={} yield={:?} consumed={} time_after={}",
                    unit.raw(),
                    yield_reason,
                    consumed_cost.raw(),
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
                    TracedBlockReason::DmaWait => "DmaWait",
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
                    TracedWakeReason::Timer => "Timer",
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
            TraceRecord::HostInvariantBreak { reason } => {
                let reason_str = match reason {
                    TracedInvariantBreakReason::Unspecified => "Unspecified",
                };
                println!("{i:4}  HostInvariantBreak reason={reason_str}");
            }
            TraceRecord::SyscallEntered {
                unit,
                num,
                args,
                disposition,
            } => {
                let disposition_str = match disposition {
                    TracedSyscallDisposition::Implemented => "Implemented",
                    TracedSyscallDisposition::Unsupported => "Unsupported",
                    TracedSyscallDisposition::UnresolvedImport => "UnresolvedImport",
                    TracedSyscallDisposition::Malformed => "Malformed",
                    TracedSyscallDisposition::Hypercall => "Hypercall",
                    TracedSyscallDisposition::TimerFastPath => "TimerFastPath",
                    TracedSyscallDisposition::NoSuchSyscall => "NoSuchSyscall",
                };
                println!(
                    "{i:4}  SyscallEntered     unit={} num=0x{num:x} disposition={disposition_str} \
                     args=[0x{:x},0x{:x},0x{:x},0x{:x},0x{:x},0x{:x},0x{:x},0x{:x}]",
                    unit.raw(),
                    args[0],
                    args[1],
                    args[2],
                    args[3],
                    args[4],
                    args[5],
                    args[6],
                    args[7],
                );
            }
            TraceRecord::ReservedRegionRead {
                unit,
                step,
                addr,
                len,
                hits,
            } => {
                println!(
                    "{i:4}  ReservedRegionRead unit={} step={step} addr=0x{addr:x} len={len} hits={hits}",
                    unit.raw(),
                );
            }
            TraceRecord::SyscallReturned { unit, code, time } => {
                println!(
                    "{i:4}  SyscallReturned    unit={} code=0x{code:x} time={}",
                    unit.raw(),
                    time.raw()
                );
            }
            TraceRecord::HostWrite {
                writer,
                space,
                addr,
                len,
                reservations_cleared,
            } => {
                println!(
                    "{i:4}  HostWrite          writer={writer:?} space={space} \
                     addr=0x{addr:x} len={len} cleared={reservations_cleared}"
                );
            }
            TraceRecord::StateHashScheme { ppu, checkpoint } => {
                println!(
                    "{i:4}  StateHashScheme    ppu=0x{ppu:016x} checkpoint=0x{checkpoint:016x}"
                );
            }
        }
    }
    println!("--- {count} records total ---");
    Ok(())
}
