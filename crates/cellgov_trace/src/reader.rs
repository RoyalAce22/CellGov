//! Iterator decoding a byte slice produced by [`crate::writer::TraceWriter`].
//!
//! On decode failure the iterator yields `Some(Err(...))` once and then `None`
//! forever -- no recovery, no skip-ahead.

use crate::record::{DecodeError, TraceRecord};

/// Iterator over records encoded in a trace byte buffer.
pub struct TraceReader<'a> {
    bytes: &'a [u8],
    pos: usize,
    failed: bool,
}

impl<'a> TraceReader<'a> {
    /// Construct a reader over `bytes`.
    #[inline]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            failed: false,
        }
    }

    /// Bytes consumed so far.
    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }
}

impl Iterator for TraceReader<'_> {
    type Item = Result<TraceRecord, DecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        if self.pos >= self.bytes.len() {
            return None;
        }
        match TraceRecord::decode(&self.bytes[self.pos..]) {
            Ok((record, n)) => {
                self.pos += n;
                Some(Ok(record))
            }
            Err(e) => {
                self.failed = true;
                Some(Err(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::StateHash;
    use crate::record::{HashCheckpointKind, TracedYieldReason};
    use crate::writer::TraceWriter;
    use cellgov_event::UnitId;
    use cellgov_time::{Budget, Epoch, GuestTicks};

    fn make_writer() -> TraceWriter {
        let mut w = TraceWriter::new();
        w.record(&TraceRecord::UnitScheduled {
            unit: UnitId::new(0),
            granted_budget: Budget::new(10),
            time: GuestTicks::ZERO,
            epoch: Epoch::ZERO,
        });
        w.record(&TraceRecord::StepCompleted {
            unit: UnitId::new(0),
            yield_reason: TracedYieldReason::BudgetExhausted,
            consumed_budget: Budget::new(10),
            time_after: GuestTicks::new(10),
        });
        w.record(&TraceRecord::CommitApplied {
            unit: UnitId::new(0),
            writes_committed: 2,
            effects_deferred: 1,
            fault_discarded: false,
            epoch_after: Epoch::new(1),
        });
        w.record(&TraceRecord::StateHashCheckpoint {
            kind: HashCheckpointKind::CommittedMemory,
            hash: StateHash::new(0x1234),
        });
        w
    }

    #[test]
    fn reader_iterates_writer_output() {
        let w = make_writer();
        let r = TraceReader::new(w.bytes());
        let records: Vec<TraceRecord> = r.collect::<Result<Vec<_>, _>>().expect("decode all");
        assert_eq!(records.len(), 4);
        match &records[0] {
            TraceRecord::UnitScheduled { unit, .. } => assert_eq!(*unit, UnitId::new(0)),
            other => panic!("wrong first record: {other:?}"),
        }
        match &records[3] {
            TraceRecord::StateHashCheckpoint { hash, .. } => {
                assert_eq!(hash.raw(), 0x1234)
            }
            other => panic!("wrong last record: {other:?}"),
        }
    }

    #[test]
    fn empty_buffer_yields_nothing() {
        let mut r = TraceReader::new(&[]);
        assert!(r.next().is_none());
    }

    #[test]
    fn truncated_buffer_surfaces_error_then_stops() {
        let w = make_writer();
        let bytes = w.bytes();
        let truncated = &bytes[..40];
        let mut r = TraceReader::new(truncated);
        assert!(matches!(
            r.next(),
            Some(Ok(TraceRecord::UnitScheduled { .. }))
        ));
        assert!(matches!(r.next(), Some(Err(_))));
        assert!(r.next().is_none());
    }

    #[test]
    fn position_advances_with_each_record() {
        let w = make_writer();
        let mut r = TraceReader::new(w.bytes());
        let _ = r.next().unwrap().unwrap();
        assert_eq!(r.position(), 33);
        let _ = r.next().unwrap().unwrap();
        assert_eq!(r.position(), 33 + 26);
    }

    #[test]
    fn writer_reader_full_roundtrip_preserves_records() {
        let original = vec![
            TraceRecord::UnitScheduled {
                unit: UnitId::new(1),
                granted_budget: Budget::new(7),
                time: GuestTicks::new(3),
                epoch: Epoch::new(2),
            },
            TraceRecord::StepCompleted {
                unit: UnitId::new(1),
                yield_reason: TracedYieldReason::Finished,
                consumed_budget: Budget::new(7),
                time_after: GuestTicks::new(10),
            },
            TraceRecord::CommitApplied {
                unit: UnitId::new(1),
                writes_committed: 0,
                effects_deferred: 0,
                fault_discarded: true,
                epoch_after: Epoch::new(3),
            },
        ];
        let mut w = TraceWriter::new();
        for r in &original {
            w.record(r);
        }
        let r = TraceReader::new(w.bytes());
        let decoded: Vec<TraceRecord> = r.collect::<Result<Vec<_>, _>>().expect("decode all");
        assert_eq!(decoded, original);
    }
}
