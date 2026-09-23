//! Checks exact finite-domain accounting and ordered worker reduction.

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};

use cellgov_fuzz::{
    finite_partition_bounds, reduce_finite_results, sweep_finite, FiniteCase, FinitePartitionError,
    FiniteSweepError, FiniteVerdict,
};
use cellgov_spu::fuzz::{encoding_execution_is_supported, generation_descriptors};

#[test]
fn empty_uneven_and_oversubscribed_partitions_count_each_case_once() {
    for (length, workers) in [(0, 1), (1, 7), (17, 1), (17, 3), (17, 7), (17, 42)] {
        let domain = (0..length).collect::<Vec<_>>();
        let calls = (0..length).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>();
        let mut delivered = Vec::new();
        let report = sweep_finite(
            &domain,
            workers,
            None,
            |&index| {
                calls[index].fetch_add(1, Ordering::SeqCst);
                Ok::<_, io::Error>(if index % 3 == 0 {
                    FiniteVerdict::Refused
                } else {
                    FiniteVerdict::Accepted(index * 2)
                })
            },
            |case| {
                delivered.push(case.index());
                Ok::<_, io::Error>(())
            },
        )
        .expect("the bounded sweep must finish");
        assert_eq!(delivered, domain);
        assert_eq!(report.total as usize, length);
        assert!(report.is_clean());
        assert_eq!(
            report.accepted + report.refused + report.panics,
            report.total
        );
        assert_eq!(report.cases.len(), length);
        assert!(calls
            .iter()
            .all(|counter| counter.load(Ordering::SeqCst) == 1));
    }
}

#[test]
fn worker_count_does_not_change_ordered_results() {
    let domain = (0..23).collect::<Vec<_>>();
    let run = |workers| {
        sweep_finite(
            &domain,
            workers,
            None,
            |&index| Ok::<_, io::Error>(FiniteVerdict::Accepted(index + 1)),
            |_| Ok::<_, io::Error>(()),
        )
        .expect("sweep must finish")
    };
    assert_eq!(run(1), run(3));
    assert_eq!(run(3), run(50));
}

#[test]
fn target_panic_is_one_classified_case_without_fallback_execution() {
    let domain = [0usize, 1, 2];
    let calls = (0..3).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>();
    let report = sweep_finite(
        &domain,
        2,
        None,
        |&index| {
            calls[index].fetch_add(1, Ordering::SeqCst);
            if index == 1 {
                panic!("seeded target panic");
            }
            Ok::<_, io::Error>(FiniteVerdict::Accepted(index))
        },
        |_| Ok::<_, io::Error>(()),
    )
    .expect("target panic belongs to the report");
    assert_eq!((report.accepted, report.refused, report.panics), (2, 0, 1));
    assert!(!report.is_clean());
    assert!(matches!(
        &report.cases[1],
        FiniteCase::Panicked { index: 1, .. }
    ));
    assert!(calls
        .iter()
        .all(|counter| counter.load(Ordering::SeqCst) == 1));
}

#[test]
fn cancellation_processes_only_the_declared_prefix_and_is_not_clean_success() {
    let domain = [0usize, 1, 2, 3, 4];
    let calls = (0..5).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>();
    let mut delivered = Vec::new();
    let error = sweep_finite(
        &domain,
        3,
        Some(2),
        |&index| {
            calls[index].fetch_add(1, Ordering::SeqCst);
            Ok::<_, io::Error>(FiniteVerdict::Accepted(index))
        },
        |case| {
            delivered.push(case.index());
            Ok::<_, io::Error>(())
        },
    )
    .expect_err("partial sweep must not look complete");
    assert!(matches!(
        error,
        FiniteSweepError::Cancelled {
            processed: 2,
            total: 5
        }
    ));
    assert_eq!(delivered, [0, 1]);
    assert_eq!(
        calls
            .iter()
            .map(|counter| counter.load(Ordering::SeqCst))
            .collect::<Vec<_>>(),
        [1, 1, 0, 0, 0]
    );
    let zero = sweep_finite(
        &domain,
        3,
        Some(0),
        |_| -> Result<FiniteVerdict<usize>, io::Error> {
            panic!("zero cancellation must not call target")
        },
        |_| Ok::<_, io::Error>(()),
    )
    .expect_err("zero cancellation remains explicit");
    assert!(matches!(
        zero,
        FiniteSweepError::Cancelled {
            processed: 0,
            total: 5
        }
    ));
    let full = sweep_finite(
        &domain,
        3,
        Some(domain.len()),
        |&index| Ok::<_, io::Error>(FiniteVerdict::Accepted(index)),
        |_| Ok::<_, io::Error>(()),
    )
    .expect("end boundary is full completion");
    assert_eq!(full.accepted, domain.len() as u64);
}

#[test]
fn target_and_sink_failures_retain_typed_indices_and_never_claim_success() {
    let domain = [0usize, 1, 2, 3];
    let mut before_error = Vec::new();
    let target_error = sweep_finite(
        &domain,
        3,
        None,
        |&index| {
            if index == 2 {
                return Err(io::Error::other("seeded target failure"));
            }
            Ok(FiniteVerdict::Accepted(index))
        },
        |case| {
            before_error.push(case.index());
            Ok::<_, io::Error>(())
        },
    )
    .expect_err("worker failure must propagate");
    assert!(matches!(
        target_error,
        FiniteSweepError::Target { index: 2, .. }
    ));
    assert!(
        before_error.is_empty(),
        "failed workers must not publish partial sink results"
    );

    let mut sink_calls = Vec::new();
    let sink_error = sweep_finite(
        &domain,
        2,
        None,
        |&index| Ok::<_, io::Error>(FiniteVerdict::Accepted(index)),
        |case| {
            sink_calls.push(case.index());
            if case.index() == 2 {
                Err(io::Error::other("seeded sink failure"))
            } else {
                Ok(())
            }
        },
    )
    .expect_err("sink refusal must propagate");
    assert!(matches!(
        sink_error,
        FiniteSweepError::Sink { index: 2, .. }
    ));
    assert_eq!(sink_calls, [0, 1, 2]);

    let panic = sweep_finite(
        &domain,
        2,
        None,
        |&index| Ok::<_, io::Error>(FiniteVerdict::Accepted(index)),
        |case| {
            if case.index() == 1 {
                panic!("seeded sink panic");
            }
            Ok::<_, io::Error>(())
        },
    )
    .expect_err("a sink panic must not unwind the sweep");
    assert!(matches!(panic, FiniteSweepError::SinkPanicked { index: 1 }));
}

#[test]
fn invalid_scheduling_refuses_before_any_target_call() {
    let domain = [0usize, 1, 2];
    let calls = AtomicUsize::new(0);
    for (workers, cancellation) in [(0, None), (2, Some(4))] {
        let error = sweep_finite(
            &domain,
            workers,
            cancellation,
            |&index| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, io::Error>(FiniteVerdict::Accepted(index))
            },
            |_| Ok::<_, io::Error>(()),
        )
        .expect_err("invalid plan must refuse");
        assert!(matches!(
            error,
            FiniteSweepError::ZeroWorkers | FiniteSweepError::CancellationOutOfRange { .. }
        ));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn interpreter_descriptors_supply_a_finite_domain_without_a_second_opcode_grammar() {
    // [Veggalam2016 p:581 s:Abstract] Useful interpreter inputs must be valid yet reach varied behavior.
    let words = generation_descriptors()
        .into_iter()
        .map(|descriptor| descriptor.canonical_word)
        .collect::<Vec<_>>();
    let report = sweep_finite(
        &words,
        4,
        None,
        |&raw| {
            let instruction =
                cellgov_spu::decode::decode(raw).expect("descriptor-owned word must decode");
            Ok::<_, io::Error>(if encoding_execution_is_supported(raw) {
                FiniteVerdict::Accepted(instruction.fuzz_descriptor().kind)
            } else {
                FiniteVerdict::Refused
            })
        },
        |_| Ok::<_, io::Error>(()),
    )
    .expect("typed descriptor domain must sweep");
    assert_eq!(report.total as usize, words.len());
    assert!(
        report.is_clean(),
        "descriptor sweep cannot hide target panics"
    );
    assert!(report.accepted > 0);
    assert!(
        report.refused > 0,
        "unsupported SPU forms must remain explicit"
    );
    assert_eq!(
        report.accepted + report.refused + report.panics,
        report.total
    );
    assert_eq!(report.cases.len(), words.len());
}

#[test]
fn missing_duplicate_and_short_worker_outputs_refuse_before_sink_publication() {
    let first = Some(vec![
        FiniteCase::Accepted {
            index: 0,
            value: 10u32,
        },
        FiniteCase::Refused { index: 1 },
    ]);
    for (second, expected_failure) in [
        (None, "missing"),
        (
            Some(vec![
                FiniteCase::Accepted {
                    index: 3,
                    value: 12,
                },
                FiniteCase::Refused { index: 3 },
            ]),
            "duplicate",
        ),
        (
            Some(vec![FiniteCase::Accepted {
                index: 2,
                value: 12,
            }]),
            "short",
        ),
    ] {
        let mut delivered = Vec::new();
        let error = reduce_finite_results::<u32, io::Error, io::Error>(
            4,
            vec![first.clone(), second],
            |case| {
                delivered.push(case.index());
                Ok(())
            },
        )
        .expect_err("missing or overlapping cases must not be clean");
        assert!(match expected_failure {
            "missing" => matches!(error, FiniteSweepError::MissingWorker { worker: 1 }),
            "duplicate" => matches!(
                error,
                FiniteSweepError::InvalidWorkerCase {
                    worker: 1,
                    expected: 2,
                    found: 3
                }
            ),
            "short" => matches!(
                error,
                FiniteSweepError::InvalidWorkerCase {
                    worker: 1,
                    expected: 4,
                    found: 3
                }
            ),
            _ => false,
        });
        assert!(
            delivered.is_empty(),
            "no partial sink result for {expected_failure}"
        );
    }
}

#[test]
fn published_partition_bounds_cover_each_case_once_without_host_scheduling() {
    for (total, workers) in [(0, 1), (1, 3), (17, 3), (17, 19)] {
        let mut visited = Vec::new();
        for worker in 0..workers {
            visited.extend(
                finite_partition_bounds(total, workers, worker)
                    .expect("worker must belong to partition"),
            );
        }
        assert_eq!(visited, (0..total).collect::<Vec<_>>());
    }
    assert_eq!(
        finite_partition_bounds(3, 0, 0),
        Err(FinitePartitionError::ZeroWorkers)
    );
    assert_eq!(
        finite_partition_bounds(3, 2, 2),
        Err(FinitePartitionError::InvalidWorker {
            worker: 2,
            workers: 2
        })
    );
}
