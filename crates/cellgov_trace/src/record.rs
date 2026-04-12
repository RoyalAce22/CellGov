//! Structured trace record types and their binary encoding.
//!
//! The brief mandates that the trace format is binary from day one.
//! Records are encoded directly into a `Vec<u8>` by the writer and
//! decoded out of a `&[u8]` by the reader -- there is no intermediate
//! Rust-value buffer that would later need to be "ported to binary".
//!
//! ## Wire format
//!
//! Each record is a 1-byte tag followed by a fixed-length variant
//! payload. All multi-byte integers are little-endian. The tag and
//! per-variant field layout below are part of the binary trace
//! contract: do not reorder fields, do not change tags, do not change
//! the discriminants of any helper enums (`YieldReason`,
//! `HashCheckpointKind`). New record variants must be appended at the
//! end with new tag values strictly greater than the current maximum.
//!
//! There is no length field after the tag because the current record
//! set is all fixed-size. When the first variable-size record lands
//! (e.g. a record carrying a payload byte slice), the encoding will
//! grow a length field at that variant only -- the fixed-size variants
//! stay binary-compatible.
//!
//! ## Variants and tags
//!
//! - `0x00 UnitScheduled`     -- 1 + 8 + 8 + 8 + 8 = 33 bytes
//! - `0x01 StepCompleted`     -- 1 + 8 + 1 + 8 + 8 = 26 bytes
//! - `0x02 CommitApplied`     -- 1 + 8 + 4 + 4 + 1 + 8 = 26 bytes
//! - `0x03 StateHashCheckpoint` -- 1 + 1 + 8 = 10 bytes
//! - `0x04 EffectEmitted`     -- 1 + 8 + 4 + 1 = 14 bytes
//! - `0x05 UnitBlocked`       -- 1 + 8 + 1 = 10 bytes
//! - `0x06 UnitWoken`         -- 1 + 8 + 1 = 10 bytes

use crate::hash::StateHash;
use crate::level::TraceLevel;
use cellgov_event::UnitId;
use cellgov_time::{Budget, Epoch, GuestTicks};

/// Yield reasons as the trace records them.
///
/// This is a parallel enum to `cellgov_exec::YieldReason` because the
/// trace crate must not depend on `cellgov_exec` (which sits above it
/// in the workspace DAG -- `effects --> exec`, `effects --> trace`).
/// The encoded discriminants here intentionally match those of
/// `cellgov_exec::YieldReason` so a future bridge layer can map between
/// the two without a translation table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TracedYieldReason {
    BudgetExhausted = 0,
    MailboxAccess = 1,
    DmaSubmitted = 2,
    DmaWait = 3,
    WaitingSync = 4,
    Syscall = 5,
    InterruptBoundary = 6,
    Fault = 7,
    Finished = 8,
}

impl TracedYieldReason {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::BudgetExhausted,
            1 => Self::MailboxAccess,
            2 => Self::DmaSubmitted,
            3 => Self::DmaWait,
            4 => Self::WaitingSync,
            5 => Self::Syscall,
            6 => Self::InterruptBoundary,
            7 => Self::Fault,
            8 => Self::Finished,
            _ => return None,
        })
    }
}

/// Which piece of state a [`TraceRecord::StateHashCheckpoint`] hashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum HashCheckpointKind {
    /// Hash of committed guest memory.
    CommittedMemory = 0,
    /// Hash of the runnable queue.
    RunnableQueue = 1,
    /// Hash of all sync object states.
    SyncState = 2,
    /// Hash of unit statuses.
    UnitStatus = 3,
}

impl HashCheckpointKind {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::CommittedMemory,
            1 => Self::RunnableQueue,
            2 => Self::SyncState,
            3 => Self::UnitStatus,
            _ => return None,
        })
    }
}

/// Why a unit was blocked, as the trace records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TracedBlockReason {
    /// Blocked by a `WaitOnEvent` effect.
    WaitOnEvent = 0,
    /// Blocked by a `MailboxReceiveAttempt` on an empty mailbox.
    MailboxEmpty = 1,
}

impl TracedBlockReason {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::WaitOnEvent,
            1 => Self::MailboxEmpty,
            _ => return None,
        })
    }
}

/// Why a unit was woken, as the trace records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TracedWakeReason {
    /// Woken by a `WakeUnit` effect from another unit.
    WakeEffect = 0,
    /// Woken by a DMA completion firing.
    DmaCompletion = 1,
}

impl TracedWakeReason {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::WakeEffect,
            1 => Self::DmaCompletion,
            _ => return None,
        })
    }
}

/// Effect kinds as the trace records them.
///
/// Parallel enum to `cellgov_effects::Effect` because the trace crate
/// must not depend on `cellgov_effects` (the workspace DAG runs
/// effects -> trace, not the other way around). The encoded
/// discriminants here intentionally match the source enum's variant
/// order so a future bridge layer can map between the two without a
/// translation table.
///
/// Discriminants are part of the binary trace contract. Do not reorder,
/// do not insert variants in the middle, do not change the explicit
/// values. New effect kinds must be appended at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TracedEffectKind {
    SharedWriteIntent = 0,
    MailboxSend = 1,
    MailboxReceiveAttempt = 2,
    DmaEnqueue = 3,
    WaitOnEvent = 4,
    WakeUnit = 5,
    SignalUpdate = 6,
    FaultRaised = 7,
    TraceMarker = 8,
}

impl TracedEffectKind {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::SharedWriteIntent,
            1 => Self::MailboxSend,
            2 => Self::MailboxReceiveAttempt,
            3 => Self::DmaEnqueue,
            4 => Self::WaitOnEvent,
            5 => Self::WakeUnit,
            6 => Self::SignalUpdate,
            7 => Self::FaultRaised,
            8 => Self::TraceMarker,
            _ => return None,
        })
    }
}

/// Why decoding a trace record stream failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The byte stream ended in the middle of a record.
    Truncated,
    /// The tag byte does not name a known record variant.
    UnknownTag(u8),
    /// A `YieldReason` discriminant byte was out of range.
    UnknownYieldReason(u8),
    /// A `HashCheckpointKind` discriminant byte was out of range.
    UnknownHashKind(u8),
    /// The `fault_discarded` flag in `CommitApplied` was neither 0 nor 1.
    InvalidBool(u8),
    /// A `TracedEffectKind` discriminant byte was out of range.
    UnknownEffectKind(u8),
    /// A `TracedBlockReason` discriminant byte was out of range.
    UnknownBlockReason(u8),
    /// A `TracedWakeReason` discriminant byte was out of range.
    UnknownWakeReason(u8),
}

/// A single structured trace record.
///
/// The variants here are the records the runtime can produce as of
/// this slice. New variants will be added as new runtime capabilities
/// land (mailbox routing, DMA completion, sync wakes, etc.). Each new
/// variant must use a strictly greater tag than the current maximum
/// to preserve binary compatibility with existing traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TraceRecord {
    /// The scheduler selected a unit and granted it a budget.
    UnitScheduled {
        unit: UnitId,
        granted_budget: Budget,
        time: GuestTicks,
        epoch: Epoch,
    },
    /// A unit's `run_until_yield` returned.
    StepCompleted {
        unit: UnitId,
        yield_reason: TracedYieldReason,
        consumed_budget: Budget,
        time_after: GuestTicks,
    },
    /// The commit pipeline finished processing a step's effects.
    CommitApplied {
        unit: UnitId,
        writes_committed: u32,
        effects_deferred: u32,
        fault_discarded: bool,
        epoch_after: Epoch,
    },
    /// A state hash was captured at a controlled checkpoint.
    StateHashCheckpoint {
        kind: HashCheckpointKind,
        hash: StateHash,
    },
    /// A unit emitted an effect during its step. Recorded once per
    /// effect, in emission order, with `sequence` running 0..N within
    /// the step. Carries the effect kind only -- effect payloads
    /// (write bytes, mailbox messages, DMA descriptors) are not in the
    /// trace at this slice; that is its own future addition.
    EffectEmitted {
        unit: UnitId,
        sequence: u32,
        kind: TracedEffectKind,
    },
    /// A unit's status was overridden to `Blocked` by the commit
    /// pipeline. Emitted once per block transition, after CommitApplied.
    UnitBlocked {
        unit: UnitId,
        reason: TracedBlockReason,
    },
    /// A unit's status was overridden to `Runnable` by the commit
    /// pipeline or a DMA completion. Emitted once per wake transition,
    /// after CommitApplied.
    UnitWoken {
        unit: UnitId,
        reason: TracedWakeReason,
    },
}

const TAG_UNIT_SCHEDULED: u8 = 0x00;
const TAG_STEP_COMPLETED: u8 = 0x01;
const TAG_COMMIT_APPLIED: u8 = 0x02;
const TAG_STATE_HASH_CHECKPOINT: u8 = 0x03;
const TAG_EFFECT_EMITTED: u8 = 0x04;
const TAG_UNIT_BLOCKED: u8 = 0x05;
const TAG_UNIT_WOKEN: u8 = 0x06;

impl TraceRecord {
    /// The category this record belongs to. Used by readers and
    /// filters that only care about a subset of the trace.
    pub fn level(&self) -> TraceLevel {
        match self {
            TraceRecord::UnitScheduled { .. }
            | TraceRecord::StepCompleted { .. }
            | TraceRecord::UnitBlocked { .. }
            | TraceRecord::UnitWoken { .. } => TraceLevel::Scheduling,
            TraceRecord::CommitApplied { .. } => TraceLevel::Commits,
            TraceRecord::StateHashCheckpoint { .. } => TraceLevel::Hashes,
            TraceRecord::EffectEmitted { .. } => TraceLevel::Effects,
        }
    }

    /// Append this record's binary encoding to `buf`.
    pub fn encode(&self, buf: &mut Vec<u8>) {
        match self {
            TraceRecord::UnitScheduled {
                unit,
                granted_budget,
                time,
                epoch,
            } => {
                buf.push(TAG_UNIT_SCHEDULED);
                write_u64(buf, unit.raw());
                write_u64(buf, granted_budget.raw());
                write_u64(buf, time.raw());
                write_u64(buf, epoch.raw());
            }
            TraceRecord::StepCompleted {
                unit,
                yield_reason,
                consumed_budget,
                time_after,
            } => {
                buf.push(TAG_STEP_COMPLETED);
                write_u64(buf, unit.raw());
                buf.push(*yield_reason as u8);
                write_u64(buf, consumed_budget.raw());
                write_u64(buf, time_after.raw());
            }
            TraceRecord::CommitApplied {
                unit,
                writes_committed,
                effects_deferred,
                fault_discarded,
                epoch_after,
            } => {
                buf.push(TAG_COMMIT_APPLIED);
                write_u64(buf, unit.raw());
                write_u32(buf, *writes_committed);
                write_u32(buf, *effects_deferred);
                buf.push(if *fault_discarded { 1 } else { 0 });
                write_u64(buf, epoch_after.raw());
            }
            TraceRecord::StateHashCheckpoint { kind, hash } => {
                buf.push(TAG_STATE_HASH_CHECKPOINT);
                buf.push(*kind as u8);
                write_u64(buf, hash.raw());
            }
            TraceRecord::EffectEmitted {
                unit,
                sequence,
                kind,
            } => {
                buf.push(TAG_EFFECT_EMITTED);
                write_u64(buf, unit.raw());
                write_u32(buf, *sequence);
                buf.push(*kind as u8);
            }
            TraceRecord::UnitBlocked { unit, reason } => {
                buf.push(TAG_UNIT_BLOCKED);
                write_u64(buf, unit.raw());
                buf.push(*reason as u8);
            }
            TraceRecord::UnitWoken { unit, reason } => {
                buf.push(TAG_UNIT_WOKEN);
                write_u64(buf, unit.raw());
                buf.push(*reason as u8);
            }
        }
    }

    /// Decode the next record from `bytes`, returning the record and
    /// the number of bytes consumed.
    pub fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let mut pos = 0usize;
        let tag = read_u8(bytes, &mut pos)?;
        let record = match tag {
            TAG_UNIT_SCHEDULED => {
                let unit = UnitId::new(read_u64(bytes, &mut pos)?);
                let granted_budget = Budget::new(read_u64(bytes, &mut pos)?);
                let time = GuestTicks::new(read_u64(bytes, &mut pos)?);
                let epoch = Epoch::new(read_u64(bytes, &mut pos)?);
                TraceRecord::UnitScheduled {
                    unit,
                    granted_budget,
                    time,
                    epoch,
                }
            }
            TAG_STEP_COMPLETED => {
                let unit = UnitId::new(read_u64(bytes, &mut pos)?);
                let yr_byte = read_u8(bytes, &mut pos)?;
                let yield_reason = TracedYieldReason::from_u8(yr_byte)
                    .ok_or(DecodeError::UnknownYieldReason(yr_byte))?;
                let consumed_budget = Budget::new(read_u64(bytes, &mut pos)?);
                let time_after = GuestTicks::new(read_u64(bytes, &mut pos)?);
                TraceRecord::StepCompleted {
                    unit,
                    yield_reason,
                    consumed_budget,
                    time_after,
                }
            }
            TAG_COMMIT_APPLIED => {
                let unit = UnitId::new(read_u64(bytes, &mut pos)?);
                let writes_committed = read_u32(bytes, &mut pos)?;
                let effects_deferred = read_u32(bytes, &mut pos)?;
                let flag = read_u8(bytes, &mut pos)?;
                let fault_discarded = match flag {
                    0 => false,
                    1 => true,
                    other => return Err(DecodeError::InvalidBool(other)),
                };
                let epoch_after = Epoch::new(read_u64(bytes, &mut pos)?);
                TraceRecord::CommitApplied {
                    unit,
                    writes_committed,
                    effects_deferred,
                    fault_discarded,
                    epoch_after,
                }
            }
            TAG_STATE_HASH_CHECKPOINT => {
                let kind_byte = read_u8(bytes, &mut pos)?;
                let kind = HashCheckpointKind::from_u8(kind_byte)
                    .ok_or(DecodeError::UnknownHashKind(kind_byte))?;
                let hash = StateHash::new(read_u64(bytes, &mut pos)?);
                TraceRecord::StateHashCheckpoint { kind, hash }
            }
            TAG_EFFECT_EMITTED => {
                let unit = UnitId::new(read_u64(bytes, &mut pos)?);
                let sequence = read_u32(bytes, &mut pos)?;
                let kind_byte = read_u8(bytes, &mut pos)?;
                let kind = TracedEffectKind::from_u8(kind_byte)
                    .ok_or(DecodeError::UnknownEffectKind(kind_byte))?;
                TraceRecord::EffectEmitted {
                    unit,
                    sequence,
                    kind,
                }
            }
            TAG_UNIT_BLOCKED => {
                let unit = UnitId::new(read_u64(bytes, &mut pos)?);
                let reason_byte = read_u8(bytes, &mut pos)?;
                let reason = TracedBlockReason::from_u8(reason_byte)
                    .ok_or(DecodeError::UnknownBlockReason(reason_byte))?;
                TraceRecord::UnitBlocked { unit, reason }
            }
            TAG_UNIT_WOKEN => {
                let unit = UnitId::new(read_u64(bytes, &mut pos)?);
                let reason_byte = read_u8(bytes, &mut pos)?;
                let reason = TracedWakeReason::from_u8(reason_byte)
                    .ok_or(DecodeError::UnknownWakeReason(reason_byte))?;
                TraceRecord::UnitWoken { unit, reason }
            }
            other => return Err(DecodeError::UnknownTag(other)),
        };
        Ok((record, pos))
    }
}

// Encoding helpers. Internal-only; exposed via TraceRecord::encode/decode.

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn read_u8(bytes: &[u8], pos: &mut usize) -> Result<u8, DecodeError> {
    let v = *bytes.get(*pos).ok_or(DecodeError::Truncated)?;
    *pos += 1;
    Ok(v)
}

fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, DecodeError> {
    let slice: [u8; 4] = bytes
        .get(*pos..*pos + 4)
        .ok_or(DecodeError::Truncated)?
        .try_into()
        .map_err(|_| DecodeError::Truncated)?;
    *pos += 4;
    Ok(u32::from_le_bytes(slice))
}

fn read_u64(bytes: &[u8], pos: &mut usize) -> Result<u64, DecodeError> {
    let slice: [u8; 8] = bytes
        .get(*pos..*pos + 8)
        .ok_or(DecodeError::Truncated)?
        .try_into()
        .map_err(|_| DecodeError::Truncated)?;
    *pos += 8;
    Ok(u64::from_le_bytes(slice))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(r: TraceRecord) {
        let mut buf = Vec::new();
        r.encode(&mut buf);
        let (decoded, n) = TraceRecord::decode(&buf).expect("decode");
        assert_eq!(decoded, r);
        assert_eq!(n, buf.len());
    }

    #[test]
    fn unit_scheduled_roundtrip() {
        roundtrip(TraceRecord::UnitScheduled {
            unit: UnitId::new(7),
            granted_budget: Budget::new(100),
            time: GuestTicks::new(42),
            epoch: Epoch::new(3),
        });
    }

    #[test]
    fn step_completed_roundtrip_each_yield_reason() {
        let reasons = [
            TracedYieldReason::BudgetExhausted,
            TracedYieldReason::MailboxAccess,
            TracedYieldReason::DmaSubmitted,
            TracedYieldReason::DmaWait,
            TracedYieldReason::WaitingSync,
            TracedYieldReason::Syscall,
            TracedYieldReason::InterruptBoundary,
            TracedYieldReason::Fault,
            TracedYieldReason::Finished,
        ];
        for r in reasons {
            roundtrip(TraceRecord::StepCompleted {
                unit: UnitId::new(1),
                yield_reason: r,
                consumed_budget: Budget::new(50),
                time_after: GuestTicks::new(100),
            });
        }
    }

    #[test]
    fn commit_applied_roundtrip() {
        roundtrip(TraceRecord::CommitApplied {
            unit: UnitId::new(2),
            writes_committed: 5,
            effects_deferred: 3,
            fault_discarded: false,
            epoch_after: Epoch::new(7),
        });
        roundtrip(TraceRecord::CommitApplied {
            unit: UnitId::new(2),
            writes_committed: 0,
            effects_deferred: 0,
            fault_discarded: true,
            epoch_after: Epoch::new(7),
        });
    }

    #[test]
    fn state_hash_checkpoint_roundtrip_each_kind() {
        let kinds = [
            HashCheckpointKind::CommittedMemory,
            HashCheckpointKind::RunnableQueue,
            HashCheckpointKind::SyncState,
            HashCheckpointKind::UnitStatus,
        ];
        for k in kinds {
            roundtrip(TraceRecord::StateHashCheckpoint {
                kind: k,
                hash: StateHash::new(0xdead_beef_cafe_babe),
            });
        }
    }

    #[test]
    fn effect_emitted_roundtrip_each_kind() {
        let kinds = [
            TracedEffectKind::SharedWriteIntent,
            TracedEffectKind::MailboxSend,
            TracedEffectKind::MailboxReceiveAttempt,
            TracedEffectKind::DmaEnqueue,
            TracedEffectKind::WaitOnEvent,
            TracedEffectKind::WakeUnit,
            TracedEffectKind::SignalUpdate,
            TracedEffectKind::FaultRaised,
            TracedEffectKind::TraceMarker,
        ];
        for (i, k) in kinds.into_iter().enumerate() {
            roundtrip(TraceRecord::EffectEmitted {
                unit: UnitId::new(3),
                sequence: i as u32,
                kind: k,
            });
        }
    }

    #[test]
    fn effect_emitted_discriminants_locked() {
        // Pinned to match cellgov_effects::Effect variant order. If
        // either side reorders, replay against an existing trace
        // breaks; this test catches local drift before that happens.
        assert_eq!(TracedEffectKind::SharedWriteIntent as u8, 0);
        assert_eq!(TracedEffectKind::MailboxSend as u8, 1);
        assert_eq!(TracedEffectKind::MailboxReceiveAttempt as u8, 2);
        assert_eq!(TracedEffectKind::DmaEnqueue as u8, 3);
        assert_eq!(TracedEffectKind::WaitOnEvent as u8, 4);
        assert_eq!(TracedEffectKind::WakeUnit as u8, 5);
        assert_eq!(TracedEffectKind::SignalUpdate as u8, 6);
        assert_eq!(TracedEffectKind::FaultRaised as u8, 7);
        assert_eq!(TracedEffectKind::TraceMarker as u8, 8);
    }

    #[test]
    fn unknown_effect_kind_returns_error() {
        let mut buf = vec![TAG_EFFECT_EMITTED];
        write_u64(&mut buf, 0);
        write_u32(&mut buf, 0);
        buf.push(99);
        assert_eq!(
            TraceRecord::decode(&buf),
            Err(DecodeError::UnknownEffectKind(99))
        );
    }

    #[test]
    fn level_classification() {
        let scheduled = TraceRecord::UnitScheduled {
            unit: UnitId::new(0),
            granted_budget: Budget::new(0),
            time: GuestTicks::ZERO,
            epoch: Epoch::ZERO,
        };
        let step = TraceRecord::StepCompleted {
            unit: UnitId::new(0),
            yield_reason: TracedYieldReason::Finished,
            consumed_budget: Budget::new(0),
            time_after: GuestTicks::ZERO,
        };
        let commit = TraceRecord::CommitApplied {
            unit: UnitId::new(0),
            writes_committed: 0,
            effects_deferred: 0,
            fault_discarded: false,
            epoch_after: Epoch::ZERO,
        };
        let hash = TraceRecord::StateHashCheckpoint {
            kind: HashCheckpointKind::CommittedMemory,
            hash: StateHash::ZERO,
        };
        let effect = TraceRecord::EffectEmitted {
            unit: UnitId::new(0),
            sequence: 0,
            kind: TracedEffectKind::SharedWriteIntent,
        };
        assert_eq!(scheduled.level(), TraceLevel::Scheduling);
        assert_eq!(step.level(), TraceLevel::Scheduling);
        assert_eq!(commit.level(), TraceLevel::Commits);
        assert_eq!(hash.level(), TraceLevel::Hashes);
        assert_eq!(effect.level(), TraceLevel::Effects);
    }

    #[test]
    fn truncated_input_returns_error() {
        let r = TraceRecord::UnitScheduled {
            unit: UnitId::new(1),
            granted_budget: Budget::new(1),
            time: GuestTicks::ZERO,
            epoch: Epoch::ZERO,
        };
        let mut buf = Vec::new();
        r.encode(&mut buf);
        // Drop the last byte: should fail to decode.
        let truncated = &buf[..buf.len() - 1];
        assert_eq!(TraceRecord::decode(truncated), Err(DecodeError::Truncated));
    }

    #[test]
    fn unknown_tag_returns_error() {
        let bad = [0xff_u8];
        assert_eq!(
            TraceRecord::decode(&bad),
            Err(DecodeError::UnknownTag(0xff))
        );
    }

    #[test]
    fn unknown_yield_reason_returns_error() {
        let mut buf = vec![TAG_STEP_COMPLETED];
        write_u64(&mut buf, 0); // unit
        buf.push(99); // bogus yield reason
        write_u64(&mut buf, 0); // consumed
        write_u64(&mut buf, 0); // time_after
        assert_eq!(
            TraceRecord::decode(&buf),
            Err(DecodeError::UnknownYieldReason(99))
        );
    }

    #[test]
    fn unknown_hash_kind_returns_error() {
        let mut buf = vec![TAG_STATE_HASH_CHECKPOINT];
        buf.push(99); // bogus kind
        write_u64(&mut buf, 0);
        assert_eq!(
            TraceRecord::decode(&buf),
            Err(DecodeError::UnknownHashKind(99))
        );
    }

    #[test]
    fn invalid_bool_returns_error() {
        let mut buf = vec![TAG_COMMIT_APPLIED];
        write_u64(&mut buf, 0);
        write_u32(&mut buf, 0);
        write_u32(&mut buf, 0);
        buf.push(2); // not 0 or 1
        write_u64(&mut buf, 0);
        assert_eq!(TraceRecord::decode(&buf), Err(DecodeError::InvalidBool(2)));
    }

    #[test]
    fn fixed_sizes_match_documentation() {
        // Verify the wire-size table in the module doc comment.
        let mut buf = Vec::new();
        TraceRecord::UnitScheduled {
            unit: UnitId::new(0),
            granted_budget: Budget::new(0),
            time: GuestTicks::ZERO,
            epoch: Epoch::ZERO,
        }
        .encode(&mut buf);
        assert_eq!(buf.len(), 33);

        buf.clear();
        TraceRecord::StepCompleted {
            unit: UnitId::new(0),
            yield_reason: TracedYieldReason::BudgetExhausted,
            consumed_budget: Budget::new(0),
            time_after: GuestTicks::ZERO,
        }
        .encode(&mut buf);
        assert_eq!(buf.len(), 26);

        buf.clear();
        TraceRecord::CommitApplied {
            unit: UnitId::new(0),
            writes_committed: 0,
            effects_deferred: 0,
            fault_discarded: false,
            epoch_after: Epoch::ZERO,
        }
        .encode(&mut buf);
        assert_eq!(buf.len(), 26);

        buf.clear();
        TraceRecord::StateHashCheckpoint {
            kind: HashCheckpointKind::CommittedMemory,
            hash: StateHash::ZERO,
        }
        .encode(&mut buf);
        assert_eq!(buf.len(), 10);

        buf.clear();
        TraceRecord::EffectEmitted {
            unit: UnitId::new(0),
            sequence: 0,
            kind: TracedEffectKind::SharedWriteIntent,
        }
        .encode(&mut buf);
        assert_eq!(buf.len(), 14);

        buf.clear();
        TraceRecord::UnitBlocked {
            unit: UnitId::new(0),
            reason: TracedBlockReason::WaitOnEvent,
        }
        .encode(&mut buf);
        assert_eq!(buf.len(), 10);

        buf.clear();
        TraceRecord::UnitWoken {
            unit: UnitId::new(0),
            reason: TracedWakeReason::WakeEffect,
        }
        .encode(&mut buf);
        assert_eq!(buf.len(), 10);
    }

    #[test]
    fn unit_blocked_roundtrip_each_reason() {
        let reasons = [
            TracedBlockReason::WaitOnEvent,
            TracedBlockReason::MailboxEmpty,
        ];
        for r in reasons {
            roundtrip(TraceRecord::UnitBlocked {
                unit: UnitId::new(5),
                reason: r,
            });
        }
    }

    #[test]
    fn unit_woken_roundtrip_each_reason() {
        let reasons = [
            TracedWakeReason::WakeEffect,
            TracedWakeReason::DmaCompletion,
        ];
        for r in reasons {
            roundtrip(TraceRecord::UnitWoken {
                unit: UnitId::new(5),
                reason: r,
            });
        }
    }

    #[test]
    fn unknown_block_reason_returns_error() {
        let mut buf = vec![TAG_UNIT_BLOCKED];
        write_u64(&mut buf, 0);
        buf.push(99);
        assert_eq!(
            TraceRecord::decode(&buf),
            Err(DecodeError::UnknownBlockReason(99))
        );
    }

    #[test]
    fn unknown_wake_reason_returns_error() {
        let mut buf = vec![TAG_UNIT_WOKEN];
        write_u64(&mut buf, 0);
        buf.push(99);
        assert_eq!(
            TraceRecord::decode(&buf),
            Err(DecodeError::UnknownWakeReason(99))
        );
    }

    #[test]
    fn blocked_and_woken_are_scheduling_level() {
        let blocked = TraceRecord::UnitBlocked {
            unit: UnitId::new(0),
            reason: TracedBlockReason::WaitOnEvent,
        };
        let woken = TraceRecord::UnitWoken {
            unit: UnitId::new(0),
            reason: TracedWakeReason::DmaCompletion,
        };
        assert_eq!(blocked.level(), TraceLevel::Scheduling);
        assert_eq!(woken.level(), TraceLevel::Scheduling);
    }
}
