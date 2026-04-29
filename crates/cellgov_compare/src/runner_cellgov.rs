//! CellGov runner adapter: produces `Observation` values from scenarios
//! (`observe`) or from long-running boots (`observe_from_boot`). The two
//! paths are separate because the testkit runner has no notion of
//! process-exit, hard faults, or HLE-driven termination that a boot
//! reports.

use crate::observation::{
    NamedMemoryRegion, Observation, ObservationMetadata, ObservedEvent, ObservedEventKind,
    ObservedHashes, ObservedOutcome,
};
use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome, ScenarioResult};
use cellgov_trace::{TraceReader, TraceRecord, TracedEffectKind, TracedWakeReason};

/// Memory region to extract from final committed memory (guest address space).
#[derive(Debug, Clone)]
pub struct RegionDescriptor {
    /// Region name for the observation.
    pub name: String,
    /// Guest address of the region start.
    pub addr: u64,
    /// Size in bytes.
    pub size: u64,
}

/// How a long-running boot terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootOutcome {
    /// Guest reached `sys_process_exit` cleanly.
    ProcessExit,
    /// PPU raised a hard fault (decode error, unimplemented, etc).
    Fault,
    /// Max-step cap reached without termination.
    MaxSteps,
    /// First PPU write into the RSX command region was attempted;
    /// used as a cross-runner checkpoint for titles whose attract-mode
    /// loops never exit on their own.
    RsxWriteCheckpoint,
    /// A step retired with its PC equal to a caller-supplied checkpoint PC.
    PcReached(u64),
    /// Internal time counter overflowed. Distinct from `Fault` so a pair
    /// yielding `TimeOverflow` on one side and `Fault` on the other is
    /// not miscategorized as "both faulted".
    TimeOverflow,
}

/// Build an `Observation` from a completed boot run.
///
/// State hashes are `None`: the boot path does not retain the per-step
/// hashes the scenario runner accumulates.
///
/// `tty_log` is the captured `sys_tty_write` byte stream from the
/// boot's `Lv2Host`; pass an empty slice when the caller has no TTY
/// data to record.
///
/// Outcome mapping:
/// - `ProcessExit`, `RsxWriteCheckpoint`, `PcReached` -> `Completed`
/// - `MaxSteps`, `TimeOverflow` -> `Timeout`
/// - `Fault` -> `Fault`
pub fn observe_from_boot(
    final_memory: &[u8],
    outcome: BootOutcome,
    steps_taken: usize,
    regions: &[RegionDescriptor],
    tty_log: &[u8],
) -> Observation {
    let observed_outcome = match outcome {
        BootOutcome::ProcessExit => ObservedOutcome::Completed,
        BootOutcome::Fault => ObservedOutcome::Fault,
        BootOutcome::MaxSteps => ObservedOutcome::Timeout,
        BootOutcome::RsxWriteCheckpoint => ObservedOutcome::Completed,
        BootOutcome::PcReached(_) => ObservedOutcome::Completed,
        BootOutcome::TimeOverflow => ObservedOutcome::Timeout,
    };

    let memory_regions = regions
        .iter()
        .map(|desc| {
            let start = desc.addr as usize;
            let end = start.saturating_add(desc.size as usize);
            let data = if start <= final_memory.len() && end <= final_memory.len() {
                final_memory[start..end].to_vec()
            } else {
                vec![0u8; desc.size as usize]
            };
            NamedMemoryRegion {
                name: desc.name.clone(),
                addr: desc.addr,
                data,
            }
        })
        .collect();

    Observation {
        outcome: observed_outcome,
        memory_regions,
        events: Vec::new(),
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: "cellgov-boot".into(),
            steps: Some(steps_taken),
        },
        tty_log: tty_log.to_vec(),
    }
}

/// Convert a `ScenarioResult` into a normalized `Observation`.
///
/// Out-of-bounds regions are filled with zeros; the comparison layer
/// catches the mismatch.
///
/// Outcome mapping: `Stalled` -> `Completed`, `MaxStepsExceeded` -> `Timeout`.
pub fn observe(result: &ScenarioResult, regions: &[RegionDescriptor]) -> Observation {
    let outcome = match result.outcome {
        ScenarioOutcome::Stalled => ObservedOutcome::Completed,
        ScenarioOutcome::MaxStepsExceeded => ObservedOutcome::Timeout,
    };

    let memory_regions = regions
        .iter()
        .map(|desc| {
            let start = desc.addr as usize;
            let end = start.saturating_add(desc.size as usize);
            let mem = &result.final_memory;
            let data = if start <= mem.len() && end <= mem.len() {
                mem[start..end].to_vec()
            } else {
                vec![0u8; desc.size as usize]
            };
            NamedMemoryRegion {
                name: desc.name.clone(),
                addr: desc.addr,
                data,
            }
        })
        .collect();

    let events = extract_events(&result.trace_bytes);

    let state_hashes = Some(ObservedHashes {
        memory: result.final_memory_hash,
        unit_status: result.final_unit_status_hash,
        sync: result.final_sync_hash,
    });

    Observation {
        outcome,
        memory_regions,
        events,
        state_hashes,
        metadata: ObservationMetadata {
            runner: "cellgov".into(),
            steps: Some(result.steps_taken),
        },
        // Scenario runner has no LV2 host with a TTY surface; left
        // empty until we add structural-test TTY capture.
        tty_log: Vec::new(),
    }
}

/// Decode the binary trace and coalesce into semantic events.
fn extract_events(trace_bytes: &[u8]) -> Vec<ObservedEvent> {
    let mut events = Vec::new();
    let mut seq: u32 = 0;

    for record in TraceReader::new(trace_bytes).flatten() {
        let maybe = match record {
            TraceRecord::EffectEmitted { unit, kind, .. } => match kind {
                TracedEffectKind::MailboxSend => Some((ObservedEventKind::MailboxSend, unit.raw())),
                TracedEffectKind::MailboxReceiveAttempt => {
                    Some((ObservedEventKind::MailboxReceive, unit.raw()))
                }
                TracedEffectKind::DmaEnqueue => Some((ObservedEventKind::DmaComplete, unit.raw())),
                _ => None,
            },
            TraceRecord::UnitBlocked { unit, .. } => {
                Some((ObservedEventKind::UnitBlock, unit.raw()))
            }
            TraceRecord::UnitWoken { unit, reason } => {
                let kind = match reason {
                    TracedWakeReason::DmaCompletion => ObservedEventKind::DmaComplete,
                    TracedWakeReason::WakeEffect => ObservedEventKind::UnitWake,
                };
                Some((kind, unit.raw()))
            }
            _ => None,
        };

        if let Some((kind, unit)) = maybe {
            events.push(ObservedEvent {
                kind,
                unit,
                sequence: seq,
            });
            seq += 1;
        }
    }

    events
}

/// Why a determinism check failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeterminismError {
    /// The two runs produced different outcomes.
    OutcomeMismatch,
    /// The two runs produced different memory region contents.
    MemoryMismatch,
    /// The two runs produced different event sequences.
    EventMismatch,
    /// The two runs produced different state hashes.
    HashMismatch,
}

/// Run a scenario factory twice and verify both observations match;
/// returns the observation, or the first field that diverged.
pub fn observe_with_determinism_check(
    factory: impl Fn() -> ScenarioFixture,
    regions: &[RegionDescriptor],
) -> Result<Observation, DeterminismError> {
    let r1 = runner::run(factory());
    let r2 = runner::run(factory());
    let o1 = observe(&r1, regions);
    let o2 = observe(&r2, regions);

    if o1.outcome != o2.outcome {
        return Err(DeterminismError::OutcomeMismatch);
    }
    if o1.memory_regions != o2.memory_regions {
        return Err(DeterminismError::MemoryMismatch);
    }
    if o1.events != o2.events {
        return Err(DeterminismError::EventMismatch);
    }
    if o1.state_hashes != o2.state_hashes {
        return Err(DeterminismError::HashMismatch);
    }

    Ok(o1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_testkit::fixtures;
    use cellgov_testkit::runner::run;

    #[test]
    fn observe_maps_stalled_to_completed() {
        let result = run(fixtures::round_robin_fairness_scenario(2, 3));
        let obs = observe(&result, &[]);
        assert_eq!(result.outcome, ScenarioOutcome::Stalled);
        assert_eq!(obs.outcome, ObservedOutcome::Completed);
    }

    #[test]
    fn observe_carries_state_hashes() {
        let result = run(fixtures::round_robin_fairness_scenario(2, 3));
        let obs = observe(&result, &[]);
        let hashes = obs.state_hashes.unwrap();
        assert_eq!(hashes.memory, result.final_memory_hash);
        assert_eq!(hashes.unit_status, result.final_unit_status_hash);
        assert_eq!(hashes.sync, result.final_sync_hash);
    }

    #[test]
    fn observe_extracts_memory_region() {
        let result = run(fixtures::write_conflict_scenario(3));
        let desc = RegionDescriptor {
            name: "first_4_bytes".into(),
            addr: 0,
            size: 4,
        };
        let obs = observe(&result, &[desc]);
        assert_eq!(obs.memory_regions.len(), 1);
        assert_eq!(obs.memory_regions[0].name, "first_4_bytes");
        assert_eq!(obs.memory_regions[0].data.len(), 4);
    }

    #[test]
    fn observe_out_of_bounds_region_is_zeros() {
        let result = run(fixtures::round_robin_fairness_scenario(1, 1));
        let desc = RegionDescriptor {
            name: "oob".into(),
            addr: 999_999,
            size: 16,
        };
        let obs = observe(&result, &[desc]);
        assert_eq!(obs.memory_regions[0].data, vec![0u8; 16]);
    }

    #[test]
    fn observe_metadata_says_cellgov() {
        let result = run(fixtures::round_robin_fairness_scenario(1, 1));
        let obs = observe(&result, &[]);
        assert_eq!(obs.metadata.runner, "cellgov");
        assert!(obs.metadata.steps.is_some());
    }

    #[test]
    fn observe_extracts_events_from_mailbox_scenario() {
        let result = run(fixtures::mailbox_send_scenario(3));
        let obs = observe(&result, &[]);
        assert!(
            obs.events
                .iter()
                .any(|e| e.kind == ObservedEventKind::MailboxSend),
            "expected at least one MailboxSend event"
        );
    }

    #[test]
    fn observe_extracts_block_wake_from_dma_scenario() {
        let result = run(fixtures::dma_block_unblock_scenario());
        let obs = observe(&result, &[]);
        assert!(
            obs.events
                .iter()
                .any(|e| e.kind == ObservedEventKind::UnitBlock),
            "expected at least one UnitBlock event"
        );
        assert!(
            obs.events
                .iter()
                .any(|e| e.kind == ObservedEventKind::DmaComplete),
            "expected at least one DmaComplete event"
        );
    }

    #[test]
    fn observe_event_sequences_are_monotonic() {
        let result = run(fixtures::mailbox_roundtrip_scenario(0x42));
        let obs = observe(&result, &[]);
        for (i, event) in obs.events.iter().enumerate() {
            assert_eq!(event.sequence, i as u32);
        }
    }

    #[test]
    fn two_identical_runs_produce_identical_observations() {
        let r1 = run(fixtures::fake_isa_scenario());
        let r2 = run(fixtures::fake_isa_scenario());
        let regions = vec![RegionDescriptor {
            name: "shared".into(),
            addr: 0,
            size: 8,
        }];
        let o1 = observe(&r1, &regions);
        let o2 = observe(&r2, &regions);
        assert_eq!(o1.outcome, o2.outcome);
        assert_eq!(o1.memory_regions, o2.memory_regions);
        assert_eq!(o1.events, o2.events);
        assert_eq!(o1.state_hashes, o2.state_hashes);
    }

    type ScenarioFactory = Box<dyn Fn() -> cellgov_testkit::ScenarioFixture>;

    #[test]
    fn determinism_check_passes_for_all_scenarios() {
        let scenarios: Vec<(&str, ScenarioFactory)> = vec![
            (
                "fairness",
                Box::new(|| fixtures::round_robin_fairness_scenario(3, 5)),
            ),
            (
                "conflict",
                Box::new(|| fixtures::write_conflict_scenario(3)),
            ),
            (
                "mailbox",
                Box::new(|| fixtures::mailbox_roundtrip_scenario(0x42)),
            ),
            ("dma", Box::new(fixtures::dma_block_unblock_scenario)),
            ("send", Box::new(|| fixtures::mailbox_send_scenario(5))),
            ("signal", Box::new(|| fixtures::signal_update_scenario(4))),
            ("isa", Box::new(fixtures::fake_isa_scenario)),
        ];
        for (name, factory) in &scenarios {
            let result = observe_with_determinism_check(factory, &[]);
            assert!(
                result.is_ok(),
                "determinism check failed for {name}: {:?}",
                result.err()
            );
        }
    }

    #[test]
    fn observe_from_boot_maps_process_exit_to_completed() {
        let mem = vec![0u8; 16];
        let obs = observe_from_boot(&mem, BootOutcome::ProcessExit, 1000, &[], &[]);
        assert_eq!(obs.outcome, ObservedOutcome::Completed);
        assert_eq!(obs.metadata.runner, "cellgov-boot");
        assert_eq!(obs.metadata.steps, Some(1000));
        assert!(obs.state_hashes.is_none());
        assert!(obs.tty_log.is_empty());
    }

    #[test]
    fn observe_from_boot_maps_fault_and_max_steps() {
        let mem = vec![0u8; 16];
        let fault = observe_from_boot(&mem, BootOutcome::Fault, 50, &[], &[]);
        assert_eq!(fault.outcome, ObservedOutcome::Fault);
        let timeout = observe_from_boot(&mem, BootOutcome::MaxSteps, 100_000, &[], &[]);
        assert_eq!(timeout.outcome, ObservedOutcome::Timeout);
    }

    #[test]
    fn observe_from_boot_maps_pc_reached_to_completed() {
        let mem = vec![0u8; 16];
        let obs = observe_from_boot(&mem, BootOutcome::PcReached(0x10381ce8), 1402388, &[], &[]);
        assert_eq!(obs.outcome, ObservedOutcome::Completed);
        assert_eq!(obs.metadata.steps, Some(1402388));
    }

    #[test]
    fn observe_from_boot_maps_rsx_write_checkpoint_to_completed() {
        let mem = vec![0u8; 16];
        let obs = observe_from_boot(&mem, BootOutcome::RsxWriteCheckpoint, 12_345, &[], &[]);
        assert_eq!(obs.outcome, ObservedOutcome::Completed);
        assert_eq!(obs.metadata.steps, Some(12_345));
    }

    #[test]
    fn observe_from_boot_extracts_regions() {
        let mut mem = vec![0u8; 256];
        mem[0x40..0x48].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let regions = vec![
            RegionDescriptor {
                name: "text".into(),
                addr: 0x40,
                size: 8,
            },
            RegionDescriptor {
                name: "out_of_bounds".into(),
                addr: 0x400,
                size: 16,
            },
        ];
        let obs = observe_from_boot(&mem, BootOutcome::ProcessExit, 1, &regions, &[]);
        assert_eq!(obs.memory_regions.len(), 2);
        assert_eq!(obs.memory_regions[0].data, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(obs.memory_regions[1].data, vec![0u8; 16]);
    }

    #[test]
    fn observe_from_boot_passes_tty_log_through() {
        let mem = vec![0u8; 16];
        let tty = b"hello world\n";
        let obs = observe_from_boot(&mem, BootOutcome::ProcessExit, 1, &[], tty);
        assert_eq!(obs.tty_log, tty);
    }
}
