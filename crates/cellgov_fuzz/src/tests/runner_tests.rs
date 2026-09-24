//! The campaign runner, driven through a sequential host that records
//! what the run asked of it.

use super::*;

/// Runs each batch in order on this thread and records every call.
#[derive(Default)]
struct RecordingHost {
    planned: Vec<u64>,
    started: Vec<(u64, u64, u64)>,
    finished: Vec<u64>,
    failures: Vec<String>,
    /// Batches the host lets start before its deadline passes.
    batches_before_deadline: Option<usize>,
}

impl CampaignHost for RecordingHost {
    fn expired(&self) -> bool {
        self.batches_before_deadline
            .is_some_and(|limit| self.started.len() >= limit)
    }

    fn run_batch(
        &mut self,
        target: FuzzTarget,
        configs: Vec<FuzzConfig>,
    ) -> Result<Vec<FuzzRun>, WorkerFailure> {
        Ok(configs
            .into_iter()
            .map(|config| target.run(config))
            .collect())
    }

    fn planned(&mut self, cases: u64) {
        self.planned.push(cases);
    }

    fn batch_started(&mut self, first: u64, count: u64, findings: u64) {
        self.started.push((first, count, findings));
    }

    fn batch_finished(&mut self, count: u64) {
        self.finished.push(count);
    }

    fn failed(&mut self, failure: CampaignFailure) {
        self.failures.push(format!("{failure:?}"));
    }
}

fn request(artifacts_dir: &std::path::Path) -> CampaignRequest {
    CampaignRequest {
        target: FuzzTarget::PpuInstruction,
        campaign_version: crate::CAMPAIGN_VERSION.0,
        seed: 7,
        strategy: GenerationStrategy::Structured,
        first: 0,
        count: 100,
        replay_case: None,
        shard: 0,
        shards: 1,
        workers: 1,
        cancel_after: None,
        finding_limit: crate::DEFAULT_MAX_FINDINGS,
        sequence_words: None,
        reduction: None,
        artifacts_dir: artifacts_dir.to_path_buf(),
        reference: ArtifactReference::Local,
        deadline_ms: None,
        progress: false,
    }
}

/// A 100-case range is two batches, and the two together count what one
/// engine run over the whole range counts.
#[test]
fn a_two_batch_campaign_runs_through_the_library_and_adds_up_to_one_run() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("runner_two_batches");
    let plan = request(&scratch).plan().expect("plans");
    let mut host = RecordingHost::default();
    let run = run_campaign(&plan, &mut host).expect("runs");

    assert_eq!(host.planned, vec![100]);
    assert_eq!(
        host.started
            .iter()
            .map(|&(first, count, _)| (first, count))
            .collect::<Vec<_>>(),
        vec![(0, 64), (64, 36)]
    );
    assert_eq!(host.finished, vec![64, 36]);
    assert_eq!(run.offset, 100);
    assert!(!run.summary.cancelled);

    let whole = FuzzTarget::PpuInstruction.run(plan.config());
    assert_eq!(run.summary.cases, whole.report.cases);
    assert_eq!(run.summary.eligible, whole.report.eligible_cases);
    assert_eq!(run.summary.finding_counts, whole.report.finding_counts);
    assert_eq!(run.summary.artifacts.len(), run.summary.findings() as usize);
    assert!(run.summary.artifacts.iter().all(|record| record.stored));
    assert!(host.failures.is_empty(), "{:?}", host.failures);
    assert_eq!(
        run.outcome(),
        if run.summary.findings() > 0 {
            CampaignOutcome::Findings
        } else {
            CampaignOutcome::Clean
        }
    );
}

/// The run asks the deadline before each batch, and a run it ends reports
/// itself cancelled.
#[test]
fn a_deadline_before_the_second_batch_leaves_the_run_cancelled() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("runner_deadline");
    let plan = request(&scratch).plan().expect("plans");
    let mut host = RecordingHost {
        batches_before_deadline: Some(1),
        ..RecordingHost::default()
    };
    let run = run_campaign(&plan, &mut host).expect("runs");
    assert_eq!(run.offset, 64);
    assert!(run.summary.cancelled);
    assert_eq!(run.outcome(), CampaignOutcome::Cancelled);
}

/// A store failure reaches the host once per finding, leaves each record
/// unstored, and does not stop the run.
#[test]
fn every_unstored_finding_is_told_to_the_host_and_ranks_the_run() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("runner_unstored");
    let occupied = scratch.join("occupied");
    std::fs::write(&occupied, b"file").expect("blocking file");
    let mut blocked = request(&occupied);
    blocked.count = 8;
    let plan = blocked.plan().expect("plans");
    let _guard = crate::seeded::seed(crate::seeded::SeededDefect::ExecutorPanic);
    let mut host = RecordingHost::default();
    let run = run_campaign(&plan, &mut host).expect("runs");

    assert!(
        !run.summary.artifacts.is_empty(),
        "the premise needs a finding"
    );
    assert!(run.summary.artifacts.iter().all(|record| !record.stored));
    assert_eq!(host.failures.len(), run.summary.artifacts.len());
    assert!(run.evidence_not_stored);
    assert_eq!(run.offset, 8);
    assert_eq!(run.outcome(), CampaignOutcome::EvidenceNotStored);
}

#[test]
fn a_request_no_run_could_serve_is_refused_before_it_runs() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("runner_refusals");
    let base = request(&scratch);
    let refused = |edit: &dyn Fn(&mut CampaignRequest)| {
        let mut request = base.clone();
        edit(&mut request);
        request.plan().expect_err("refused")
    };
    assert!(matches!(
        refused(&|r| r.finding_limit = 0),
        CampaignError::FindingLimit
    ));
    assert!(matches!(
        refused(&|r| r.finding_limit = MAX_RETAINED_FINDINGS + 1),
        CampaignError::FindingLimit
    ));
    assert!(matches!(
        refused(&|r| r.sequence_words = Some(8)),
        CampaignError::SequenceWords
    ));
    assert!(matches!(refused(&|r| r.shard = 1), CampaignError::Shard));
    assert!(matches!(
        refused(&|r| r.count = 0),
        CampaignError::Range { first: 0, count: 0 }
    ));
    assert!(matches!(
        refused(&|r| r.cancel_after = Some(101)),
        CampaignError::CancelAfter
    ));
    assert!(matches!(
        refused(&|r| {
            r.reduction = Some(ReductionRequest {
                policy: crate::reduce::ReductionPolicy::Deterministic,
                budget: 0,
            })
        }),
        CampaignError::ReductionBudget
    ));
    assert!(matches!(
        refused(&|r| r.workers = 0),
        CampaignError::Workers
    ));
    assert!(matches!(
        refused(&|r| r.workers = MAX_CAMPAIGN_WORKERS + 1),
        CampaignError::Workers
    ));
    assert!(matches!(
        refused(&|r| r.deadline_ms = Some(0)),
        CampaignError::Deadline
    ));
    assert!(matches!(
        refused(&|r| r.artifacts_dir = "//".into()),
        CampaignError::ArtifactsDir
    ));
    let mut widest = base.clone();
    widest.workers = MAX_CAMPAIGN_WORKERS;
    assert!(widest.plan().is_ok());
    let mut sequence = base.clone();
    sequence.target = FuzzTarget::PpuSequence;
    sequence.sequence_words = Some(8);
    assert!(sequence.plan().is_ok());
}

#[test]
fn host_workers_preserve_the_selected_shard_across_batches() {
    let first = 100u64;
    let count = 130u64;
    let mut observed = std::collections::BTreeSet::new();
    for offset in [0u64, 64, 128] {
        let batch = (count - offset).min(64);
        for worker in 0..3u32 {
            let index = worker_shard_index(1, 2, worker, 6, (offset % 6) as u32)
                .expect("valid worker partition");
            let schedule = CampaignSchedule {
                cases: CaseRange {
                    first: first + offset,
                    count: batch,
                },
                shard: CampaignShard { index, count: 6 },
                cancellation: None,
            };
            for case in schedule.case_indices().expect("bounded schedule") {
                assert!(observed.insert(case), "case {case} was assigned twice");
            }
        }
    }
    let expected = (first..first + count)
        .filter(|case| (case - first) % 2 == 1)
        .collect();
    assert_eq!(observed, expected);
}
