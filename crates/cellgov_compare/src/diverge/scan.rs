//! Streaming per-step state-hash divergence scanner.
//!
//! Walks two binary trace streams and compares their per-step hash
//! records stream by stream: the `PpuStateHash` records as one stream,
//! and the `SpuStateHash` records of each SPU unit as one stream each.
//! Two runs can interleave their units differently, so each stream is
//! compared on its own. The traces are consumed as iterators and never
//! materialized: memory is one entry per stream, and time is one pass
//! over each trace per stream.
//!
//! [Armstrong2019 p:71:24 s:7] A trace comparison between two
//! simulators checks that they execute matching instructions and make
//! matching register writes; here the PC is checked before the hash.
//!
//! `PpuStateHash` covers scalar integer state only, so a PPU step is
//! the first *scalar-visible* disagreement. Two runs diverging in a
//! float or vector register agree here until that value reaches a covered
//! register, which can be arbitrarily far downstream. `SpuStateHash`
//! covers the SPU's registers, so the same caveat holds for the SPU
//! state it leaves out: channels, signals and the stopped state.

use std::collections::BTreeSet;
use std::fmt;

use cellgov_event::UnitId;
use cellgov_trace::{TraceReader, TraceRecord};

use crate::trace_decode::TraceDecodeError;

/// One per-step hash stream of a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StateStream {
    /// The `PpuStateHash` records. They name no unit.
    Ppu,
    /// The `SpuStateHash` records of one SPU unit.
    Spu(UnitId),
}

impl StateStream {
    /// The kind of unit the stream's records come from.
    pub fn kind(self) -> StateHashKind {
        match self {
            Self::Ppu => StateHashKind::Ppu,
            Self::Spu(_) => StateHashKind::Spu,
        }
    }
}

impl fmt::Display for StateStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ppu => f.write_str("ppu"),
            Self::Spu(unit) => write!(f, "spu:{}", unit.raw()),
        }
    }
}

/// The kind of unit a state hash covers, which names its scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateHashKind {
    /// A PPU state hash.
    Ppu,
    /// An SPU state hash.
    Spu,
}

impl fmt::Display for StateHashKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ppu => "ppu",
            Self::Spu => "spu",
        })
    }
}

/// Which field disagreed at the first differing step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergeField {
    /// PCs differ at this step.
    Pc,
    /// PCs match but state hashes differ at this step.
    Hash,
}

/// Outcome of comparing two per-step state-hash streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DivergeReport {
    /// The two traces hold hashes of one kind under two schemes; see
    /// [`trace_scheme`]. The scan compares no record and claims no
    /// divergence.
    SchemeMismatch {
        /// The kind of hash whose schemes differ.
        kind: StateHashKind,
        /// Scheme id of side A.
        a: u64,
        /// Scheme id of side B.
        b: u64,
    },
    /// All `count` records matched pairwise and every stream ended on
    /// both sides.
    Identical {
        /// Records matched on each side, over every stream.
        count: u64,
    },
    /// Both sides reached `step` of `stream` but disagreed on `field`.
    Differs {
        /// The stream that disagreed first.
        stream: StateStream,
        /// 0-based index within `stream` of the first scalar-visible
        /// disagreement; see the module docs for why that is not always
        /// the first divergence.
        step: u64,
        /// PC on side A.
        a_pc: u64,
        /// PC on side B.
        b_pc: u64,
        /// State hash on side A.
        a_hash: u64,
        /// State hash on side B.
        b_hash: u64,
        /// Which field broke first.
        field: DivergeField,
    },
    /// One side of `stream` ended before the other; `common_count` of
    /// its records matched.
    LengthDiffers {
        /// The stream whose sides ran to two lengths.
        stream: StateStream,
        /// Records of `stream` matched before either side ended.
        common_count: u64,
        /// Records of `stream` in side A.
        a_count: u64,
        /// Records of `stream` in side B.
        b_count: u64,
    },
    /// A trace stopped decoding before the scan finished. Not a verdict
    /// on the runs: the `common_count` records before the failure
    /// matched, and nothing past it was compared.
    CorruptTrace {
        /// Records matched before the failure, over every stream.
        common_count: u64,
        /// Decode failure on side A, if that side failed.
        a_error: Option<TraceDecodeError>,
        /// Decode failure on side B, if that side failed.
        b_error: Option<TraceDecodeError>,
    },
}

/// The scheme ids a trace stream names for its state hashes.
///
/// A tool that compares one record kind across two streams checks that
/// kind's id first, and reports a scheme mismatch when the ids differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceSchemes {
    /// Scheme id of the stream's `PpuStateHash` records.
    pub ppu: u64,
    /// Scheme id of the stream's `StateHashCheckpoint` records.
    pub checkpoint: u64,
    /// Scheme id of the stream's `SpuStateHash` records.
    pub spu: u64,
}

impl TraceSchemes {
    /// The schemes of a stream that has no `StateHashScheme` record.
    pub const UNSTAMPED: Self = Self {
        ppu: cellgov_ppu::state::FNV1A_SCHEME_ID,
        checkpoint: crate::observation::LEGACY_CHECKPOINT_HASH_SCHEME,
        // A stream without the record predates SPU state hashes and
        // holds none, so no id names their scheme.
        spu: 0,
    };
}

/// The scheme ids of a stream's state hashes.
///
/// The ids come from the `StateHashScheme` record after the header. A
/// stream without that record reads as [`TraceSchemes::UNSTAMPED`]. A
/// stream whose leading records do not decode reads the same way;
/// [`diverge`] reports the decode failure for that stream.
pub fn trace_scheme(bytes: &[u8]) -> TraceSchemes {
    leading_scheme(bytes).unwrap_or(TraceSchemes::UNSTAMPED)
}

/// The scheme ids of a stream, or `None` when a leading record does not
/// decode and so the schemes are unknown.
fn leading_scheme(bytes: &[u8]) -> Option<TraceSchemes> {
    let mut reader = TraceReader::new(bytes);
    let mut first = reader.next();
    if matches!(first, Some(Ok(TraceRecord::RunIdentity { .. }))) {
        first = reader.next();
    }
    match first {
        Some(Err(_)) => None,
        Some(Ok(TraceRecord::StateHashScheme {
            ppu,
            checkpoint,
            spu,
        })) => Some(TraceSchemes {
            ppu,
            checkpoint,
            spu,
        }),
        _ => Some(TraceSchemes::UNSTAMPED),
    }
}

/// Walk two trace byte slices and report the first per-step hash
/// divergence.
///
/// Each stream is compared on its own, and the reports combine in this
/// order:
///
/// 1. A two-scheme pair of one hash kind ends the scan before it reads a
///    record: [`DivergeReport::SchemeMismatch`]. The checkpoint schemes
///    do not stop the scan, since it reads no checkpoint record.
/// 2. A disagreement in any stream: [`DivergeReport::Differs`] for the
///    one whose record comes first in side A. A stream's scan stops at a
///    decode failure, so every disagreement lies before any failure.
/// 3. A record on either side that fails to decode:
///    [`DivergeReport::CorruptTrace`], even when the record lies past
///    the other side's clean end.
/// 4. A stream whose two sides end at two lengths:
///    [`DivergeReport::LengthDiffers`], for the first such stream in
///    [`StateStream`] order.
///
/// A side whose leading records do not decode has no known scheme, so
/// the scan reports the decode failure. That failure comes before any
/// state hash, so the scan compares no hash.
pub fn diverge(a: &[u8], b: &[u8]) -> DivergeReport {
    if let (Some(a_scheme), Some(b_scheme)) = (leading_scheme(a), leading_scheme(b)) {
        for (kind, a_id, b_id) in [
            (StateHashKind::Ppu, a_scheme.ppu, b_scheme.ppu),
            (StateHashKind::Spu, a_scheme.spu, b_scheme.spu),
        ] {
            if a_id != b_id {
                return DivergeReport::SchemeMismatch {
                    kind,
                    a: a_id,
                    b: b_id,
                };
            }
        }
    }
    let mut streams = streams_in(a);
    streams.extend(streams_in(b));
    if streams.is_empty() {
        // Nothing to compare still has to decode.
        streams.insert(StateStream::Ppu);
    }
    let outcomes: Vec<(StateStream, StreamOutcome)> = streams
        .into_iter()
        .map(|stream| (stream, scan_stream(a, b, stream)))
        .collect();
    let matched: u64 = outcomes.iter().map(|(_, o)| o.matched()).sum();
    let first_differ = outcomes
        .iter()
        .filter_map(|(_, o)| match o {
            StreamOutcome::Differs { ordinal, report } => Some((*ordinal, report)),
            _ => None,
        })
        .min_by_key(|(ordinal, _)| *ordinal);
    if let Some((_, report)) = first_differ {
        return report.clone();
    }
    if let Some((a_error, b_error)) = outcomes.iter().find_map(|(_, o)| match o {
        StreamOutcome::Corrupt {
            a_error, b_error, ..
        } => Some((*a_error, *b_error)),
        _ => None,
    }) {
        return DivergeReport::CorruptTrace {
            common_count: matched,
            a_error,
            b_error,
        };
    }
    if let Some(report) = outcomes.iter().find_map(|(_, o)| match o {
        StreamOutcome::LengthDiffers(report) => Some(report.clone()),
        _ => None,
    }) {
        return report;
    }
    DivergeReport::Identical { count: matched }
}

/// How the scan of one stream ended.
enum StreamOutcome {
    /// Every record matched and both sides ended.
    Identical { matched: u64 },
    /// A record disagreed; `ordinal` is its record index in side A.
    Differs {
        ordinal: usize,
        report: DivergeReport,
    },
    /// One side ended first.
    LengthDiffers(DivergeReport),
    /// A record failed to decode after `matched` records matched.
    Corrupt {
        matched: u64,
        a_error: Option<TraceDecodeError>,
        b_error: Option<TraceDecodeError>,
    },
}

impl StreamOutcome {
    /// Records of the stream that matched.
    fn matched(&self) -> u64 {
        match self {
            Self::Identical { matched } | Self::Corrupt { matched, .. } => *matched,
            Self::Differs { report, .. } | Self::LengthDiffers(report) => match report {
                DivergeReport::Differs { step, .. } => *step,
                DivergeReport::LengthDiffers { common_count, .. } => *common_count,
                _ => 0,
            },
        }
    }
}

/// The streams a trace holds records of, read up to its end or its first
/// record that does not decode.
fn streams_in(bytes: &[u8]) -> BTreeSet<StateStream> {
    TraceReader::new(bytes)
        .map_while(Result::ok)
        .filter_map(|record| stream_of(&record).map(|(stream, ..)| stream))
        .collect()
}

/// The stream, PC and hash of a per-step hash record.
fn stream_of(record: &TraceRecord) -> Option<(StateStream, u64, u64)> {
    match *record {
        TraceRecord::PpuStateHash { pc, hash, .. } => Some((StateStream::Ppu, pc, hash.raw())),
        TraceRecord::SpuStateHash { unit, pc, hash, .. } => {
            Some((StateStream::Spu(unit), pc, hash.raw()))
        }
        _ => None,
    }
}

/// Compare the records of `stream` in two traces.
fn scan_stream(a: &[u8], b: &[u8], stream: StateStream) -> StreamOutcome {
    let mut ai = state_hash_iter(a, stream);
    let mut bi = state_hash_iter(b, stream);
    let mut step: u64 = 0;
    loop {
        let (a_next, b_next) = match (ai.next().transpose(), bi.next().transpose()) {
            (Ok(a_next), Ok(b_next)) => (a_next, b_next),
            (a_next, b_next) => {
                return StreamOutcome::Corrupt {
                    matched: step,
                    a_error: a_next.err(),
                    b_error: b_next.err(),
                }
            }
        };
        match (a_next, b_next) {
            (None, None) => return StreamOutcome::Identical { matched: step },
            (Some(_), None) => {
                return match remaining(ai) {
                    Ok(rest) => StreamOutcome::LengthDiffers(DivergeReport::LengthDiffers {
                        stream,
                        common_count: step,
                        a_count: step + 1 + rest,
                        b_count: step,
                    }),
                    Err(error) => StreamOutcome::Corrupt {
                        matched: step,
                        a_error: Some(error),
                        b_error: None,
                    },
                };
            }
            (None, Some(_)) => {
                return match remaining(bi) {
                    Ok(rest) => StreamOutcome::LengthDiffers(DivergeReport::LengthDiffers {
                        stream,
                        common_count: step,
                        a_count: step,
                        b_count: step + 1 + rest,
                    }),
                    Err(error) => StreamOutcome::Corrupt {
                        matched: step,
                        a_error: None,
                        b_error: Some(error),
                    },
                };
            }
            (Some(a_rec), Some(b_rec)) => {
                let field = if a_rec.pc != b_rec.pc {
                    Some(DivergeField::Pc)
                } else if a_rec.hash != b_rec.hash {
                    Some(DivergeField::Hash)
                } else {
                    None
                };
                if let Some(field) = field {
                    return StreamOutcome::Differs {
                        ordinal: a_rec.ordinal,
                        report: DivergeReport::Differs {
                            stream,
                            step,
                            a_pc: a_rec.pc,
                            b_pc: b_rec.pc,
                            a_hash: a_rec.hash,
                            b_hash: b_rec.hash,
                            field,
                        },
                    };
                }
                step += 1;
            }
        }
    }
}

/// Count the records of a stream left on one side after the other
/// ended, failing at the first record that does not decode.
fn remaining(
    iter: impl Iterator<Item = Result<HashRecord, TraceDecodeError>>,
) -> Result<u64, TraceDecodeError> {
    let mut count = 0;
    for record in iter {
        record?;
        count += 1;
    }
    Ok(count)
}

/// One per-step hash record and its record index in the trace.
#[derive(Debug, Clone, Copy)]
struct HashRecord {
    ordinal: usize,
    pc: u64,
    hash: u64,
}

/// Iterate the records of `stream`, skipping every other record; a
/// decode failure is yielded once, positioned by the index and byte
/// offset of the record that failed, and then the stream ends.
fn state_hash_iter(
    bytes: &[u8],
    stream: StateStream,
) -> impl Iterator<Item = Result<HashRecord, TraceDecodeError>> + '_ {
    let mut reader = TraceReader::new(bytes);
    let mut index = 0usize;
    std::iter::from_fn(move || loop {
        let offset = reader.position();
        match reader.next()? {
            Ok(record) => {
                let ordinal = index;
                index += 1;
                if let Some((s, pc, hash)) = stream_of(&record) {
                    if s == stream {
                        return Some(Ok(HashRecord { ordinal, pc, hash }));
                    }
                }
            }
            Err(source) => {
                return Some(Err(TraceDecodeError {
                    index,
                    offset,
                    source,
                }))
            }
        }
    })
}

#[cfg(test)]
#[path = "tests/scan_tests.rs"]
mod tests;
