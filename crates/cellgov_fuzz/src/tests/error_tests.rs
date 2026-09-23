use super::*;

use std::error::Error;

use cellgov_ppu::instruction::fuzz::PpuGenerationError;
use cellgov_ppu::observation::PpuObservationError;
use cellgov_spu::fuzz::SpuGenerationError;

use crate::{CampaignVersion, RetentionConfigError};

fn source_text(error: &dyn Error) -> Option<String> {
    error.source().map(ToString::to_string)
}

#[test]
fn configuration_errors_display_their_exact_text() {
    let cases: [(ConfigurationError, &str); 8] = [
        (
            ConfigurationError::UnsupportedCampaignVersion {
                found: CampaignVersion(5),
                supported: CampaignVersion(4),
            },
            "campaign version 5 does not match supported version 4",
        ),
        (
            ConfigurationError::ZeroIterations,
            "fuzz campaign must contain at least one case",
        ),
        (
            ConfigurationError::CaseRangeOverflow {
                first: u64::MAX,
                count: 2,
            },
            "case range starting at 18446744073709551615 with count 2 overflows the index space",
        ),
        (
            ConfigurationError::InvalidShard { index: 3, count: 3 },
            "campaign shard 3 is invalid for a partition of 3",
        ),
        (
            ConfigurationError::CancellationOutOfRange {
                offset: 9,
                count: 8,
            },
            "cancellation offset 9 exceeds campaign count 8",
        ),
        (
            ConfigurationError::ZeroSequenceWords,
            "fuzz sequence must contain at least one instruction",
        ),
        (
            ConfigurationError::SequenceTooLong {
                requested: 65_537,
                maximum: 65_536,
            },
            "fuzz sequence has 65537 words; target limit is 65536",
        ),
        (
            ConfigurationError::TooManyRetainedFindings {
                requested: 1_025,
                maximum: 1_024,
            },
            "fuzz campaign retains 1025 findings; limit is 1024",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
        assert_eq!(source_text(&error), None);
    }
}

#[test]
fn retention_configuration_errors_wrap_their_source() {
    let error = ConfigurationError::Retention {
        source: RetentionConfigError::ZeroCapacity,
    };

    assert_eq!(
        error.to_string(),
        "invalid case-retention configuration: case-retention capacity must be nonzero"
    );
    assert_eq!(
        source_text(&error),
        Some("case-retention capacity must be nonzero".to_owned())
    );
}

#[test]
fn generator_errors_display_their_exact_text() {
    let cases: [(GeneratorError, &str); 7] = [
        (
            GeneratorError::ConstraintAttemptsExhausted {
                target: "ppu",
                attempts: 64,
            },
            "ppu structured generation exhausted 64 constraint attempts",
        ),
        (
            GeneratorError::EmptyDescriptorRegistry { target: "spu" },
            "spu descriptor registry is empty",
        ),
        (
            GeneratorError::ParameterIndex {
                index: 3,
                length: 3,
            },
            "parameter index 3 is outside stream length 3",
        ),
        (
            GeneratorError::ZeroProbabilityDenominator,
            "generator probability denominator must be nonzero",
        ),
        (
            GeneratorError::InvalidProbability {
                numerator: 2,
                denominator: 1,
            },
            "generator probability numerator 2 exceeds denominator 1",
        ),
        (
            GeneratorError::Exhausted {
                last_raw: 0xdead_beef,
            },
            "generator exhausted its bounded decoder search at raw word 0xdeadbeef",
        ),
        (
            GeneratorError::Exhausted { last_raw: 0 },
            "generator exhausted its bounded decoder search at raw word 0x00000000",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
        assert_eq!(source_text(&error), None);
    }
}

#[test]
fn interpreter_generation_errors_convert_into_generator_errors() {
    let ppu = GeneratorError::from(PpuGenerationError::InvalidOperands);
    let spu = GeneratorError::from(SpuGenerationError::InvalidOperands);

    assert_eq!(
        ppu,
        GeneratorError::Ppu(PpuGenerationError::InvalidOperands)
    );
    assert_eq!(
        ppu.to_string(),
        "PPU structural generation failed: PPU operands are invalid for selected instruction kind"
    );
    assert_eq!(
        source_text(&ppu),
        Some("PPU operands are invalid for selected instruction kind".to_owned())
    );
    assert_eq!(
        spu,
        GeneratorError::Spu(SpuGenerationError::InvalidOperands)
    );
    assert_eq!(
        spu.to_string(),
        concat!(
            "SPU structural generation failed: ",
            "SPU operands do not encode a valid selected instruction kind"
        )
    );
    assert_eq!(
        source_text(&spu),
        Some("SPU operands do not encode a valid selected instruction kind".to_owned())
    );
}

#[test]
fn reference_disagreement_names_both_references() {
    let error = ReferenceDisagreement {
        left: "interpreter",
        right: "mirror",
    };

    assert_eq!(
        error.to_string(),
        "reference interpreter disagreed with mirror"
    );
    assert_eq!(source_text(&error), None);
}

#[test]
fn worker_failures_wrap_the_harness_failure_as_their_source() {
    let error = WorkerError::Failed {
        worker: 2,
        failure: Box::new(FuzzError::Configuration(ConfigurationError::ZeroIterations)),
    };

    assert_eq!(
        error.to_string(),
        "worker 2 failed: invalid configuration: fuzz campaign must contain at least one case"
    );
    assert_eq!(
        source_text(&error),
        Some("invalid configuration: fuzz campaign must contain at least one case".to_owned())
    );
}

#[test]
fn worker_panics_name_the_worker() {
    let error = WorkerError::Panicked { worker: 0 };

    assert_eq!(
        error.to_string(),
        "worker 0 panicked outside the target boundary"
    );
    assert_eq!(source_text(&error), None);
}

#[test]
fn synchronization_errors_name_the_poisoned_object() {
    let error = SynchronizationError { object: "findings" };

    assert_eq!(
        error.to_string(),
        "fuzz synchronization object findings was poisoned"
    );
    assert_eq!(source_text(&error), None);
}

#[test]
fn reporting_errors_name_the_sink() {
    let error = ReportingError { sink: "artifact" };

    assert_eq!(error.to_string(), "finding sink artifact rejected a report");
    assert_eq!(source_text(&error), None);
}

#[test]
fn reduction_errors_display_their_exact_text() {
    let cases: [(ReductionError, &str); 8] = [
        (
            ReductionError::InvalidCandidate {
                transform: "drop_word",
            },
            "reduction transform drop_word produced an invalid case",
        ),
        (
            ReductionError::FingerprintChanged {
                transform: "zero_operand",
            },
            "reduction transform zero_operand changed the finding fingerprint",
        ),
        (
            ReductionError::OriginalNotReproduced,
            "reduction found no finding at the original case",
        ),
        (
            ReductionError::OriginalInapplicable,
            "reduction refuses an unsupported or undefined original case",
        ),
        (
            ReductionError::NothingToReduce,
            "reduction has no case words to transform",
        ),
        (
            ReductionError::BudgetExhausted { evaluations: 17 },
            "reduction budget ended after 17 evaluations with no round settled",
        ),
        (
            ReductionError::IncompatibleReplay,
            "reduction configuration does not match the finding's replay coordinates",
        ),
        (
            ReductionError::SessionOrder { expected: "verify" },
            "reduction session expected verify",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
        assert_eq!(source_text(&error), None);
    }
}

#[test]
fn candidate_evaluation_failures_wrap_the_engine_failure_as_their_source() {
    let error = ReductionError::CandidateEvaluation {
        source: Box::new(FuzzError::Invariant(InvariantError::EmptyGeneratedSequence)),
    };

    assert_eq!(
        error.to_string(),
        concat!(
            "reduction candidate evaluation failed: fuzz harness invariant failed: ",
            "generated instruction sequence was unexpectedly empty"
        )
    );
    assert_eq!(
        source_text(&error),
        Some(
            "fuzz harness invariant failed: generated instruction sequence was unexpectedly empty"
                .to_owned()
        )
    );
}

#[test]
fn replay_version_errors_display_bare_version_numbers() {
    let error = ReplayVersionError {
        found: CampaignVersion(1),
        supported: CampaignVersion(4),
    };

    assert_eq!(
        error.to_string(),
        "replay version 1 does not match supported version 4"
    );
    assert_eq!(source_text(&error), None);
}

#[test]
fn invariant_errors_display_their_exact_text() {
    let cases: [(InvariantError, &str); 4] = [
        (
            InvariantError::CounterOverflow { counter: "cases" },
            "fuzz report counter cases overflowed",
        ),
        (
            InvariantError::ValueOutOfRange {
                value_kind: "depth",
                value: u64::MAX,
            },
            "generated depth value 18446744073709551615 is outside the supported range",
        ),
        (
            InvariantError::UnexpectedPanic {
                stage: "generation",
            },
            "fuzz harness panicked outside the target boundary during generation",
        ),
        (
            InvariantError::EmptyGeneratedSequence,
            "generated instruction sequence was unexpectedly empty",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
        assert_eq!(source_text(&error), None);
    }
}

#[test]
fn fuzz_errors_prefix_and_expose_every_wrapped_failure() {
    let cases: [(FuzzError, &str, &str); 10] = [
        (
            FuzzError::from(ConfigurationError::ZeroIterations),
            "invalid configuration: ",
            "fuzz campaign must contain at least one case",
        ),
        (
            FuzzError::from(GeneratorError::ZeroProbabilityDenominator),
            "case generation failed: ",
            "generator probability denominator must be nonzero",
        ),
        (
            FuzzError::from(ReferenceDisagreement {
                left: "a",
                right: "b",
            }),
            "reference comparison failed: ",
            "reference a disagreed with b",
        ),
        (
            FuzzError::from(WorkerError::Panicked { worker: 1 }),
            "campaign worker failed: ",
            "worker 1 panicked outside the target boundary",
        ),
        (
            FuzzError::from(SynchronizationError { object: "report" }),
            "campaign synchronization failed: ",
            "fuzz synchronization object report was poisoned",
        ),
        (
            FuzzError::from(ReportingError { sink: "stdout" }),
            "finding reporting failed: ",
            "finding sink stdout rejected a report",
        ),
        (
            FuzzError::from(ReductionError::NothingToReduce),
            "finding reduction failed: ",
            "reduction has no case words to transform",
        ),
        (
            FuzzError::from(ReplayVersionError {
                found: CampaignVersion(0),
                supported: CampaignVersion(4),
            }),
            "finding replay failed: ",
            "replay version 0 does not match supported version 4",
        ),
        (
            FuzzError::from(InvariantError::EmptyGeneratedSequence),
            "fuzz harness invariant failed: ",
            "generated instruction sequence was unexpectedly empty",
        ),
        (
            FuzzError::from(PpuObservationError::FaultEffectWithoutFault),
            "PPU observation failed: ",
            "FaultRaised reached a non-faulting PPU observation",
        ),
    ];

    for (error, prefix, inner) in cases {
        assert_eq!(error.to_string(), format!("{prefix}{inner}"));
        assert_eq!(source_text(&error), Some(inner.to_owned()));
    }
}

#[test]
fn fuzz_error_conversions_keep_the_wrapped_value() {
    assert_eq!(
        FuzzError::from(ConfigurationError::ZeroSequenceWords),
        FuzzError::Configuration(ConfigurationError::ZeroSequenceWords)
    );
    assert_eq!(
        FuzzError::from(PpuObservationError::FaultEffectWithoutFault),
        FuzzError::PpuObservation(PpuObservationError::FaultEffectWithoutFault)
    );
}
