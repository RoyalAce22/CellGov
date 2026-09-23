use super::*;

use std::io;

fn accepted(index: usize, value: u32) -> FiniteCase<u32> {
    FiniteCase::Accepted { index, value }
}

fn refused(index: usize) -> FiniteCase<u32> {
    FiniteCase::Refused { index }
}

fn panicked(index: usize, payload: TargetPanicPayload) -> FiniteCase<u32> {
    FiniteCase::Panicked { index, payload }
}

fn ok_sink(case: &FiniteCase<u32>) -> Result<(), io::Error> {
    let _ = case;
    Ok(())
}

fn reduce(
    total: usize,
    worker_results: Vec<Option<Vec<FiniteCase<u32>>>>,
    delivered: &mut Vec<usize>,
) -> Result<FiniteSweepReport<u32>, FiniteSweepError<io::Error, io::Error>> {
    reduce_finite_results(total, worker_results, |case| {
        delivered.push(case.index());
        Ok(())
    })
}

#[test]
fn partition_error_display_names_the_worker_and_count() {
    assert_eq!(
        FinitePartitionError::ZeroWorkers.to_string(),
        "finite partition requires at least one worker"
    );
    assert_eq!(
        FinitePartitionError::InvalidWorker {
            worker: 2,
            workers: 2
        }
        .to_string(),
        "finite worker index 2 is outside a partition of 2"
    );
}

#[test]
fn partition_bounds_give_the_remainder_to_the_lowest_workers() {
    let bounds = |total, workers| {
        (0..workers)
            .map(|worker| finite_partition_bounds(total, workers, worker).expect("valid worker"))
            .collect::<Vec<_>>()
    };
    assert_eq!(bounds(10, 4), [0..3, 3..6, 6..8, 8..10]);
    assert_eq!(bounds(3, 5), [0..1, 1..2, 2..3, 3..3, 3..3]);
    assert_eq!(bounds(0, 3), [0..0, 0..0, 0..0]);
    assert_eq!(bounds(7, 2), [0..4, 4..7]);
    assert_eq!(bounds(5, 5), [0..1, 1..2, 2..3, 3..4, 4..5]);
    assert_eq!(bounds(1, 2), [0..1, 1..1]);
}

#[test]
fn partition_bounds_refuse_zero_workers_before_the_worker_index() {
    assert_eq!(
        finite_partition_bounds(0, 0, 0),
        Err(FinitePartitionError::ZeroWorkers)
    );
    assert_eq!(
        finite_partition_bounds(3, 1, 1),
        Err(FinitePartitionError::InvalidWorker {
            worker: 1,
            workers: 1
        })
    );
    assert_eq!(
        finite_partition_bounds(3, 2, usize::MAX),
        Err(FinitePartitionError::InvalidWorker {
            worker: usize::MAX,
            workers: 2
        })
    );
    assert_eq!(
        finite_partition_bounds(3, usize::MAX, usize::MAX - 1),
        Ok(3..3)
    );
    assert_eq!(finite_partition_bounds(3, usize::MAX, 0), Ok(0..1));
}

#[test]
fn case_index_is_the_stable_index_for_every_variant() {
    assert_eq!(accepted(4, 9).index(), 4);
    assert_eq!(refused(5).index(), 5);
    assert_eq!(panicked(6, TargetPanicPayload::NonString).index(), 6);
    assert_eq!(accepted(usize::MAX, 0).index(), usize::MAX);
}

#[test]
fn report_is_clean_only_without_panics() {
    let report = FiniteSweepReport::<u32> {
        total: 3,
        accepted: 1,
        refused: 2,
        panics: 0,
        cases: Vec::new(),
    };
    assert!(report.is_clean());
    let panicked = FiniteSweepReport::<u32> {
        panics: 1,
        ..report.clone()
    };
    assert!(!panicked.is_clean());
    let empty = FiniteSweepReport::<u32> {
        total: 0,
        accepted: 0,
        refused: 0,
        ..report
    };
    assert!(empty.is_clean());
}

#[test]
fn sweep_error_display_names_every_failure() {
    type SweepError = FiniteSweepError<io::Error, io::Error>;
    let target = SweepError::Target {
        index: 3,
        source: io::Error::other("target boom"),
    };
    assert_eq!(
        target.to_string(),
        "finite sweep target failed at index 3: target boom"
    );
    assert_eq!(
        std::error::Error::source(&target).map(ToString::to_string),
        Some("target boom".to_owned())
    );
    let sink = SweepError::Sink {
        index: 4,
        source: io::Error::other("sink boom"),
    };
    assert_eq!(
        sink.to_string(),
        "finite sweep sink failed at index 4: sink boom"
    );
    assert_eq!(
        std::error::Error::source(&sink).map(ToString::to_string),
        Some("sink boom".to_owned())
    );
    let plain: [(SweepError, &str); 7] = [
        (
            SweepError::ZeroWorkers,
            "finite sweep requires at least one worker",
        ),
        (
            SweepError::CancellationOutOfRange {
                offset: 6,
                total: 5,
            },
            "finite sweep cancellation offset 6 exceeds 5 cases",
        ),
        (
            SweepError::CounterOverflow,
            "finite sweep domain size exceeds the report counter range",
        ),
        (
            SweepError::MissingWorker { worker: 2 },
            "finite sweep worker 2 returned no partition",
        ),
        (
            SweepError::InvalidWorkerCase {
                worker: 1,
                expected: 2,
                found: 3,
            },
            "finite sweep worker 1 returned case 3 instead of 2",
        ),
        (
            SweepError::SinkPanicked { index: 7 },
            "finite sweep sink panicked at index 7",
        ),
        (
            SweepError::Cancelled {
                processed: 2,
                total: 9,
            },
            "finite sweep cancelled after 2 of 9 cases",
        ),
    ];
    for (error, text) in &plain {
        assert_eq!(error.to_string(), *text);
        assert!(std::error::Error::source(error).is_none(), "{text}");
    }
}

#[test]
fn sweep_keeps_refusals_and_panics_at_their_indices_across_uneven_partitions() {
    let domain = (0..9u32).collect::<Vec<_>>();
    let target = |&raw: &u32| -> Result<FiniteVerdict<u32>, io::Error> {
        match raw {
            2 => Ok(FiniteVerdict::Refused),
            5 => panic!("seeded static"),
            7 => panic!("seeded owned {raw}"),
            _ => Ok(FiniteVerdict::Accepted(raw * 10)),
        }
    };
    let mut delivered = Vec::new();
    let report = sweep_finite(&domain, 4, None, target, |case| {
        delivered.push(case.index());
        Ok::<_, io::Error>(())
    })
    .expect("panics and refusals are classified cases");
    assert_eq!(
        report.cases,
        [
            accepted(0, 0),
            accepted(1, 10),
            refused(2),
            accepted(3, 30),
            accepted(4, 40),
            panicked(5, TargetPanicPayload::StaticStr("seeded static".to_owned())),
            accepted(6, 60),
            panicked(7, TargetPanicPayload::String("seeded owned 7".to_owned())),
            accepted(8, 80),
        ]
    );
    assert_eq!(delivered, (0..9).collect::<Vec<_>>());
    assert_eq!(
        (report.total, report.accepted, report.refused, report.panics),
        (9, 6, 1, 2)
    );
    assert!(!report.is_clean());
    let serial = sweep_finite(&domain, 1, None, target, ok_sink).expect("serial sweep");
    assert_eq!(serial, report);
}

#[test]
fn sweep_of_an_empty_domain_completes_with_or_without_a_zero_cancellation() {
    let domain: [u32; 0] = [];
    let target = |_: &u32| -> Result<FiniteVerdict<u32>, io::Error> {
        panic!("empty domain must not call the target")
    };
    for cancel in [None, Some(0)] {
        let report = sweep_finite(&domain, 5, cancel, target, ok_sink)
            .expect("empty domain has nothing to cancel");
        assert_eq!(report.total, 0);
        assert!(report.cases.is_empty());
        assert!(report.is_clean());
    }
    assert!(matches!(
        sweep_finite(&domain, 5, Some(1), target, ok_sink),
        Err(FiniteSweepError::CancellationOutOfRange {
            offset: 1,
            total: 0
        })
    ));
}

#[test]
fn sweep_reports_the_full_domain_length_after_a_cancelled_prefix() {
    let domain = (0..6u32).collect::<Vec<_>>();
    let mut delivered = Vec::new();
    let error = sweep_finite(
        &domain,
        4,
        Some(5),
        |&raw| Ok::<_, io::Error>(FiniteVerdict::Accepted(raw)),
        |case| {
            delivered.push(case.index());
            Ok::<_, io::Error>(())
        },
    )
    .expect_err("one case short of completion is cancellation");
    assert!(matches!(
        error,
        FiniteSweepError::Cancelled {
            processed: 5,
            total: 6
        }
    ));
    assert_eq!(delivered, [0, 1, 2, 3, 4]);
}

#[test]
fn reduce_refuses_out_of_order_cases_before_publishing() {
    let mut delivered = Vec::new();
    let reversed = reduce(2, vec![Some(vec![refused(1), refused(0)])], &mut delivered);
    assert!(matches!(
        reversed,
        Err(FiniteSweepError::InvalidWorkerCase {
            worker: 0,
            expected: 0,
            found: 1
        })
    ));
    let swapped = reduce(
        4,
        vec![
            Some(vec![refused(2), refused(3)]),
            Some(vec![refused(0), refused(1)]),
        ],
        &mut delivered,
    );
    assert!(matches!(
        swapped,
        Err(FiniteSweepError::InvalidWorkerCase {
            worker: 0,
            expected: 0,
            found: 2
        })
    ));
    let late = reduce(
        4,
        vec![
            Some(vec![refused(0), refused(1)]),
            Some(vec![refused(2), refused(2)]),
        ],
        &mut delivered,
    );
    assert!(matches!(
        late,
        Err(FiniteSweepError::InvalidWorkerCase {
            worker: 1,
            expected: 3,
            found: 2
        })
    ));
    let extra = reduce(
        2,
        vec![
            Some(vec![refused(0)]),
            Some(vec![refused(1)]),
            Some(vec![refused(2)]),
        ],
        &mut delivered,
    );
    assert!(matches!(
        extra,
        Err(FiniteSweepError::InvalidWorkerCase {
            worker: 2,
            expected: 2,
            found: 3
        })
    ));
    let missing_last = reduce(
        2,
        vec![Some(vec![refused(0)]), Some(vec![refused(1)]), None],
        &mut delivered,
    );
    assert!(matches!(
        missing_last,
        Err(FiniteSweepError::MissingWorker { worker: 2 })
    ));
    assert!(delivered.is_empty());
}

#[test]
fn reduce_accepts_ordered_partitions_and_counts_each_class() {
    let mut delivered = Vec::new();
    let report = reduce(
        5,
        vec![
            Some(vec![
                accepted(0, 7),
                panicked(1, TargetPanicPayload::NonString),
            ]),
            Some(vec![refused(2), accepted(3, 9)]),
            Some(vec![refused(4)]),
        ],
        &mut delivered,
    )
    .expect("ordered partitions reduce");
    assert_eq!(delivered, [0, 1, 2, 3, 4]);
    assert_eq!(
        (report.total, report.accepted, report.refused, report.panics),
        (5, 2, 2, 1)
    );
    assert_eq!(
        report.cases,
        [
            accepted(0, 7),
            panicked(1, TargetPanicPayload::NonString),
            refused(2),
            accepted(3, 9),
            refused(4),
        ]
    );
    let empty = reduce(0, vec![Some(Vec::new())], &mut delivered).expect("empty domain");
    assert_eq!(empty.total, 0);
    assert!(empty.cases.is_empty());
    let spare_worker = reduce(
        2,
        vec![
            Some(vec![refused(0)]),
            Some(vec![refused(1)]),
            Some(Vec::new()),
        ],
        &mut delivered,
    )
    .expect("a spare worker owns an empty partition");
    assert_eq!(spare_worker.refused, 2);
}

#[test]
fn reduce_refuses_an_empty_worker_list() {
    let mut delivered = Vec::new();
    assert!(matches!(
        reduce(0, Vec::new(), &mut delivered),
        Err(FiniteSweepError::ZeroWorkers)
    ));
    assert!(matches!(
        reduce(3, Vec::new(), &mut delivered),
        Err(FiniteSweepError::ZeroWorkers)
    ));
}

#[test]
fn reduce_attributes_sink_failure_and_panic_to_the_case_index() {
    let partitions = || {
        vec![
            Some(vec![refused(0), refused(1)]),
            Some(vec![refused(2), refused(3)]),
        ]
    };
    let mut delivered = Vec::new();
    let failed = reduce_finite_results::<u32, io::Error, io::Error>(4, partitions(), |case| {
        delivered.push(case.index());
        if case.index() == 3 {
            Err(io::Error::other("seeded sink failure"))
        } else {
            Ok(())
        }
    })
    .expect_err("sink refusal propagates");
    assert!(matches!(failed, FiniteSweepError::Sink { index: 3, .. }));
    assert_eq!(delivered, [0, 1, 2, 3]);
    let panicked = reduce_finite_results::<u32, io::Error, io::Error>(4, partitions(), |case| {
        if case.index() == 2 {
            panic!("seeded sink panic");
        }
        Ok(())
    })
    .expect_err("sink panic propagates");
    assert!(matches!(
        panicked,
        FiniteSweepError::SinkPanicked { index: 2 }
    ));
}

#[test]
fn decode_panic_serializes_its_payload_tag_and_refuses_unknown_fields() {
    let sample = DecodePanic {
        raw: 5,
        payload: TargetPanicPayload::StaticStr("x".to_owned()),
    };
    let json = serde_json::to_string(&sample).expect("serializes");
    assert_eq!(
        json,
        "{\"raw\":5,\"payload\":{\"kind\":\"static_str\",\"message\":\"x\"}}"
    );
    assert_eq!(
        serde_json::from_str::<DecodePanic>(&json).expect("round trip"),
        sample
    );
    let non_string = DecodePanic {
        raw: u32::MAX,
        payload: TargetPanicPayload::NonString,
    };
    let json = serde_json::to_string(&non_string).expect("serializes");
    assert_eq!(
        json,
        "{\"raw\":4294967295,\"payload\":{\"kind\":\"non_string\"}}"
    );
    assert_eq!(
        serde_json::from_str::<DecodePanic>(&json).expect("round trip"),
        non_string
    );
    assert!(serde_json::from_str::<DecodePanic>(
        "{\"raw\":5,\"payload\":{\"kind\":\"non_string\"},\"index\":0}"
    )
    .is_err());
    assert!(serde_json::from_str::<DecodePanic>("{\"raw\":5}").is_err());
}

#[test]
fn decoder_partitions_report_their_bounds_and_agree_with_the_chunked_scan() {
    let last_ppu = ppu_decode_partition(u32::MAX..=u32::MAX);
    assert_eq!((last_ppu.first, last_ppu.last), (u32::MAX, u32::MAX));
    assert_eq!(last_ppu.accepted + last_ppu.refused, 1);
    assert!(last_ppu.panics.is_empty());
    let first_spu = spu_decode_partition(0..=0);
    assert_eq!((first_spu.first, first_spu.last), (0, 0));
    assert_eq!(first_spu.accepted + first_spu.refused, 1);
    assert!(first_spu.panics.is_empty());

    let ppu = ppu_decode_partition(0..=63);
    let spu = spu_decode_partition(0..=63);
    assert_ne!(ppu.accepted, spu.accepted);
    assert_eq!(ppu.accepted + ppu.refused, 64);
    assert_eq!(spu.accepted + spu.refused, 64);
    let selected = crate::raw_decode::RawDecodeDomain::new(0, 64).expect("bounded domain");
    let ppu_scan = crate::raw_decode::scan_raw_decoder(
        crate::raw_decode::RawDecoder::Ppu,
        selected,
        8,
        2,
        None,
    )
    .expect("PPU scan");
    let spu_scan = crate::raw_decode::scan_raw_decoder(
        crate::raw_decode::RawDecoder::Spu,
        selected,
        8,
        2,
        None,
    )
    .expect("SPU scan");
    assert_eq!(
        (ppu.accepted, ppu.refused),
        (ppu_scan.accepted, ppu_scan.refused)
    );
    assert_eq!(
        (spu.accepted, spu.refused),
        (spu_scan.accepted, spu_scan.refused)
    );
}
