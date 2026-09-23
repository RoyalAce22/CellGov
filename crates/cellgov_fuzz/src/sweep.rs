//! Deterministic finite-domain sweeps and decoder partitions.

use crate::boundary::{call_harness, call_target};
use crate::TargetPanicPayload;
use std::error::Error;
use std::ops::{Range, RangeInclusive};

/// Invalid deterministic partition request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FinitePartitionError {
    /// Rejects a partition with no worker to own its cases.
    #[error("finite partition requires at least one worker")]
    ZeroWorkers,
    /// The worker index does not belong to this partition count.
    #[error("finite worker index {worker} is outside a partition of {workers}")]
    InvalidWorker {
        /// Requested worker index.
        worker: usize,
        /// Number of workers.
        workers: usize,
    },
}

/// Assigns one caller-managed worker a stable contiguous part of the domain.
///
/// # Errors
///
/// Refuses zero workers or an index outside the worker count.
pub fn finite_partition_bounds(
    total: usize,
    workers: usize,
    worker: usize,
) -> Result<Range<usize>, FinitePartitionError> {
    if workers == 0 {
        return Err(FinitePartitionError::ZeroWorkers);
    }
    if worker >= workers {
        return Err(FinitePartitionError::InvalidWorker { worker, workers });
    }
    let (start, end) = partition_bounds(total, workers, worker);
    Ok(start..end)
}

/// Classification returned by a caller-supplied finite-domain target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FiniteVerdict<T> {
    /// The target accepted the case and returned a typed value.
    Accepted(T),
    /// The target explicitly refused the case.
    Refused,
}

/// One classified case in domain order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FiniteCase<T> {
    /// Carries the target's typed result to the ordered sink.
    Accepted {
        /// Domain index.
        index: usize,
        /// Target value.
        value: T,
    },
    /// Retains the refusal without a fabricated result value.
    Refused {
        /// Domain index.
        index: usize,
    },
    /// Keeps one panic record at the original case index.
    Panicked {
        /// Domain index.
        index: usize,
        /// Classified panic payload.
        payload: TargetPanicPayload,
    },
}

impl<T> FiniteCase<T> {
    /// Returns the stable index assigned before workers start.
    pub fn index(&self) -> usize {
        match self {
            Self::Accepted { index, .. }
            | Self::Refused { index }
            | Self::Panicked { index, .. } => *index,
        }
    }
}

/// Exact counts and ordered cases from a completed finite-domain sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiniteSweepReport<T> {
    /// Number of inputs supplied by the caller.
    pub total: u64,
    /// Cases accepted by the target.
    pub accepted: u64,
    /// Cases refused by the target.
    pub refused: u64,
    /// Target panics captured without retrying the input.
    pub panics: u64,
    /// Classified cases in domain order.
    pub cases: Vec<FiniteCase<T>>,
}

impl<T> FiniteSweepReport<T> {
    /// Reports clean completion only when no target call panicked.
    pub fn is_clean(&self) -> bool {
        self.panics == 0
    }
}

/// A finite sweep that cannot report clean completion.
#[derive(Debug, thiserror::Error)]
pub enum FiniteSweepError<E: Error + 'static, S: Error + 'static> {
    /// No worker can process the domain.
    #[error("finite sweep requires at least one worker")]
    ZeroWorkers,
    /// Requested cancellation boundary exceeds the supplied domain.
    #[error("finite sweep cancellation offset {offset} exceeds {total} cases")]
    CancellationOutOfRange {
        /// Requested offset.
        offset: usize,
        /// Domain length.
        total: usize,
    },
    /// The report counter cannot represent the domain size.
    #[error("finite sweep domain size exceeds the report counter range")]
    CounterOverflow,
    /// Target returned a typed failure at this domain index.
    #[error("finite sweep target failed at index {index}: {source}")]
    Target {
        /// Domain index.
        index: usize,
        /// Preserves the target error for caller diagnosis.
        #[source]
        source: E,
    },
    /// A caller-managed worker did not return its assigned partition.
    #[error("finite sweep worker {worker} returned no partition")]
    MissingWorker {
        /// Stable worker index.
        worker: usize,
    },
    /// A caller-managed worker returned a case out of its assigned order.
    #[error("finite sweep worker {worker} returned case {found} instead of {expected}")]
    InvalidWorkerCase {
        /// Stable worker index.
        worker: usize,
        /// Expected case index.
        expected: usize,
        /// Received case index or the first omitted index.
        found: usize,
    },
    /// Sink rejected a classified case.
    #[error("finite sweep sink failed at index {index}: {source}")]
    Sink {
        /// Domain index.
        index: usize,
        /// Preserves the sink error for caller diagnosis.
        #[source]
        source: S,
    },
    /// Sink panicked outside the target-call boundary.
    #[error("finite sweep sink panicked at index {index}")]
    SinkPanicked {
        /// Domain index whose result the sink was receiving.
        index: usize,
    },
    /// Caller stopped the run after a deterministic prefix.
    #[error("finite sweep cancelled after {processed} of {total} cases")]
    Cancelled {
        /// Number of cases delivered to the sink.
        processed: usize,
        /// Full caller domain length.
        total: usize,
    },
}

/// Executes a finite domain once per case and reduces results in input order.
///
/// Evaluates partitions sequentially. A host may schedule partitions itself
/// and pass its results to [`reduce_finite_results`].
///
/// The results merge in domain order, so the report is the same whatever the
/// completion order. [Krook2023 p:5 s:4 Design and Implementation (parallel shrinking)]
///
/// # Errors
///
/// Returns a typed refusal for invalid scheduling, failed workers or targets,
/// sink failure, counter overflow, or cancellation.
pub fn sweep_finite<I, T, E, S>(
    domain: &[I],
    workers: usize,
    cancel_after: Option<usize>,
    target: impl Fn(&I) -> Result<FiniteVerdict<T>, E>,
    mut sink: impl FnMut(&FiniteCase<T>) -> Result<(), S>,
) -> Result<FiniteSweepReport<T>, FiniteSweepError<E, S>>
where
    E: Error + 'static,
    S: Error + 'static,
{
    if workers == 0 {
        return Err(FiniteSweepError::ZeroWorkers);
    }
    if cancel_after.is_some_and(|offset| offset > domain.len()) {
        return Err(FiniteSweepError::CancellationOutOfRange {
            offset: cancel_after.unwrap_or(0),
            total: domain.len(),
        });
    }
    let total = u64::try_from(domain.len()).map_err(|_| FiniteSweepError::CounterOverflow)?;
    let limit = cancel_after.unwrap_or(domain.len());
    let active_workers = workers.min(limit.max(1));
    let mut worker_results = Vec::with_capacity(active_workers);
    for worker in 0..active_workers {
        let (start, end) = partition_bounds(limit, active_workers, worker);
        let mut cases = Vec::with_capacity(end - start);
        for (offset, input) in domain[start..end].iter().enumerate() {
            let index = start + offset;
            let case = match call_target(|| target(input)) {
                Ok(Ok(FiniteVerdict::Accepted(value))) => FiniteCase::Accepted { index, value },
                Ok(Ok(FiniteVerdict::Refused)) => FiniteCase::Refused { index },
                Ok(Err(source)) => return Err(FiniteSweepError::Target { index, source }),
                Err(payload) => FiniteCase::Panicked { index, payload },
            };
            cases.push(case);
        }
        worker_results.push(Some(cases));
    }
    let mut report = reduce_finite_results(limit, worker_results, &mut sink)?;
    report.total = total;
    if limit < domain.len() {
        return Err(FiniteSweepError::Cancelled {
            processed: limit,
            total: domain.len(),
        });
    }
    Ok(report)
}

fn partition_bounds(total: usize, workers: usize, worker: usize) -> (usize, usize) {
    let start = (total / workers) * worker + (total % workers).min(worker);
    let end = (total / workers) * (worker + 1) + (total % workers).min(worker + 1);
    (start, end)
}

/// Reduces caller-managed partitions and refuses any missing or misordered case.
///
/// Checks all worker results before it sends any case to the sink.
///
/// # Errors
///
/// Returns a typed error for a lost worker, invalid case index, failed sink,
/// invalid worker count, or count overflow.
pub fn reduce_finite_results<T, E, S>(
    total: usize,
    worker_results: Vec<Option<Vec<FiniteCase<T>>>>,
    mut sink: impl FnMut(&FiniteCase<T>) -> Result<(), S>,
) -> Result<FiniteSweepReport<T>, FiniteSweepError<E, S>>
where
    E: Error + 'static,
    S: Error + 'static,
{
    if worker_results.is_empty() {
        return Err(FiniteSweepError::ZeroWorkers);
    }
    let total = u64::try_from(total).map_err(|_| FiniteSweepError::CounterOverflow)?;
    let assigned = total as usize;
    let workers = worker_results.len();
    for (worker, result) in worker_results.iter().enumerate() {
        let cases = result
            .as_ref()
            .ok_or(FiniteSweepError::MissingWorker { worker })?;
        let (start, end) = partition_bounds(assigned, workers, worker);
        if cases.len() != end - start {
            return Err(FiniteSweepError::InvalidWorkerCase {
                worker,
                expected: end,
                found: start + cases.len(),
            });
        }
        for (offset, case) in cases.iter().enumerate() {
            let expected = start + offset;
            if case.index() != expected {
                return Err(FiniteSweepError::InvalidWorkerCase {
                    worker,
                    expected,
                    found: case.index(),
                });
            }
        }
    }
    let mut report = FiniteSweepReport {
        total,
        accepted: 0,
        refused: 0,
        panics: 0,
        cases: Vec::with_capacity(assigned),
    };
    for (worker, result) in worker_results.into_iter().enumerate() {
        let cases = result.ok_or(FiniteSweepError::MissingWorker { worker })?;
        let (start, _end) = partition_bounds(assigned, workers, worker);
        for (offset, case) in cases.into_iter().enumerate() {
            let index = start + offset;
            match call_harness(|| sink(&case)) {
                Ok(Ok(())) => {}
                Ok(Err(source)) => return Err(FiniteSweepError::Sink { index, source }),
                Err(_) => return Err(FiniteSweepError::SinkPanicked { index }),
            }
            let count = match &case {
                FiniteCase::Accepted { .. } => &mut report.accepted,
                FiniteCase::Refused { .. } => &mut report.refused,
                FiniteCase::Panicked { .. } => &mut report.panics,
            };
            *count = count
                .checked_add(1)
                .ok_or(FiniteSweepError::CounterOverflow)?;
            report.cases.push(case);
        }
    }
    Ok(report)
}

/// One decoder panic captured at the target boundary.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodePanic {
    /// Raw word passed to the decoder.
    pub raw: u32,
    /// Deterministic payload classification.
    pub payload: TargetPanicPayload,
}

/// Counts decoder results for one caller-selected partition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeSweepReport {
    /// First raw word in the partition.
    pub first: u32,
    /// Last raw word in the partition.
    pub last: u32,
    /// Words accepted by the decoder.
    pub accepted: u64,
    /// Words refused by the decoder.
    pub refused: u64,
    /// Words that made the decoder panic.
    pub panics: Vec<DecodePanic>,
}

/// Checks the PPU decoder across the supplied word range.
pub fn ppu_decode_partition(words: RangeInclusive<u32>) -> DecodeSweepReport {
    run(words, |raw| crate::seeded::ppu_decode(raw).is_ok())
}

/// Checks the SPU decoder across the supplied word range.
pub fn spu_decode_partition(words: RangeInclusive<u32>) -> DecodeSweepReport {
    run(words, |raw| crate::seeded::spu_decode(raw).is_ok())
}

fn run(words: RangeInclusive<u32>, decode: impl Fn(u32) -> bool) -> DecodeSweepReport {
    let first = *words.start();
    let last = *words.end();
    let mut accepted = 0u64;
    let mut refused = 0u64;
    let mut panics = Vec::new();
    for raw in words {
        match call_target(|| decode(raw)) {
            Ok(true) => accepted += 1,
            Ok(false) => refused += 1,
            Err(payload) => panics.push(DecodePanic { raw, payload }),
        }
    }
    DecodeSweepReport {
        first,
        last,
        accepted,
        refused,
        panics,
    }
}

#[cfg(test)]
#[path = "tests/sweep_tests.rs"]
mod tests;
