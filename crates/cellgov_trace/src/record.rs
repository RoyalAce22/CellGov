//! Structured trace record types and their binary encoding.
//!
//! # Wire format
//!
//! Each record is a 1-byte tag followed by a fixed-length variant payload, all
//! multi-byte integers little-endian. Tags, per-variant field layouts, and the
//! discriminants of `TracedYieldReason`, `HashCheckpointKind`,
//! `TracedEffectKind`, `TracedBlockReason`, and `TracedWakeReason` are part of
//! the binary trace contract; new record variants append with strictly greater
//! tags. The current set is fixed-size, so there is no length field after the
//! tag.
//!
//! | Tag    | Variant               | Bytes                          |
//! |--------|-----------------------|--------------------------------|
//! | `0x00` | `UnitScheduled`       | 1 + 8 + 8 + 8 + 8 = 33         |
//! | `0x01` | `StepCompleted`       | 1 + 8 + 1 + 8 + 8 = 26         |
//! | `0x02` | `CommitApplied`       | 1 + 8 + 4 + 4 + 1 + 8 = 26     |
//! | `0x03` | `StateHashCheckpoint` | 1 + 1 + 8 = 10                 |
//! | `0x04` | `EffectEmitted`       | 1 + 8 + 4 + 1 = 14             |
//! | `0x05` | `UnitBlocked`         | 1 + 8 + 1 = 10                 |
//! | `0x06` | `UnitWoken`           | 1 + 8 + 1 = 10                 |
//! | `0x07` | `PpuStateHash`        | 1 + 8 + 8 + 8 = 25             |
//! | `0x08` | `PpuStateFull`        | 1 + 8 + 8 + 32*8 + 8 + 8 + 8 + 4 = 301 |

use crate::hash::StateHash;
use crate::level::TraceLevel;
use cellgov_event::UnitId;
use cellgov_time::{Budget, Epoch, GuestTicks, InstructionCost};

/// Mirror of `cellgov_exec::YieldReason` for the trace stream.
///
/// Discriminants must match the source enum: the trace crate cannot depend on
/// `cellgov_exec` (DAG: effects -> exec, effects -> trace), so the bridge maps
/// by raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TracedYieldReason {
    /// Unit consumed its full granted budget.
    BudgetExhausted = 0,
    /// Unit yielded for mailbox access.
    MailboxAccess = 1,
    /// Unit submitted a DMA and yielded.
    DmaSubmitted = 2,
    /// Unit yielded waiting for DMA completion.
    DmaWait = 3,
    /// Unit yielded waiting on a sync primitive.
    WaitingSync = 4,
    /// Unit yielded on a syscall.
    Syscall = 5,
    /// Unit yielded at an interrupt boundary.
    InterruptBoundary = 6,
    /// Unit yielded due to a fault.
    Fault = 7,
    /// Unit reached its terminal state.
    Finished = 8,
    /// `sc` with LEV >= 1 (CBE Handbook 11.1). PS3 usermode never issues these;
    /// distinguished from `Syscall` so a rejection cannot byte-collide with an
    /// unrelated LV2 handler returning `CELL_EINVAL`.
    Hypercall = 9,
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
            9 => Self::Hypercall,
            _ => return None,
        })
    }
}

/// Which piece of state a [`TraceRecord::StateHashCheckpoint`] hashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum HashCheckpointKind {
    /// Hash covers committed shared memory.
    CommittedMemory = 0,
    /// Hash covers the runnable-unit queue.
    RunnableQueue = 1,
    /// Hash covers sync-primitive state.
    SyncState = 2,
    /// Hash covers per-unit status flags.
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
    /// Unit blocked waiting on a sync event.
    WaitOnEvent = 0,
    /// Unit blocked because its mailbox was empty.
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
    /// Unit woken by a wake effect.
    WakeEffect = 0,
    /// Unit woken by DMA completion.
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

/// Mirror of `cellgov_effects::Effect` for the trace stream.
///
/// Discriminants must match the source variant order: the trace crate cannot
/// depend on `cellgov_effects` (DAG: effects -> trace), so the bridge maps by
/// raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TracedEffectKind {
    /// Shared-memory write intent.
    SharedWriteIntent = 0,
    /// Mailbox send.
    MailboxSend = 1,
    /// Mailbox receive attempt.
    MailboxReceiveAttempt = 2,
    /// DMA descriptor enqueued.
    DmaEnqueue = 3,
    /// Wait on a sync event.
    WaitOnEvent = 4,
    /// Wake another unit.
    WakeUnit = 5,
    /// Sync signal update.
    SignalUpdate = 6,
    /// Fault raised by the unit.
    FaultRaised = 7,
    /// User-emitted trace marker.
    TraceMarker = 8,
    /// `lwarx` / `ldarx` / `getllar`.
    ReservationAcquire = 9,
    /// `stwcx` / `stdcx` / `putllc` success.
    ConditionalStore = 10,
    /// NV406E semaphore release or report writeback.
    RsxLabelWrite = 11,
    /// NV4097 flip.
    RsxFlipRequest = 12,
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
            9 => Self::ReservationAcquire,
            10 => Self::ConditionalStore,
            11 => Self::RsxLabelWrite,
            12 => Self::RsxFlipRequest,
            _ => return None,
        })
    }
}

/// Why decoding a trace record stream failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Byte stream ended mid-record.
    Truncated,
    /// Record tag byte is not a known variant.
    UnknownTag(u8),
    /// Yield-reason byte is not a known variant.
    UnknownYieldReason(u8),
    /// Hash-checkpoint-kind byte is not a known variant.
    UnknownHashKind(u8),
    /// `fault_discarded` flag was neither 0 nor 1.
    InvalidBool(u8),
    /// Effect-kind byte is not a known variant.
    UnknownEffectKind(u8),
    /// Block-reason byte is not a known variant.
    UnknownBlockReason(u8),
    /// Wake-reason byte is not a known variant.
    UnknownWakeReason(u8),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => f.write_str("byte stream ended mid-record"),
            Self::UnknownTag(b) => write!(f, "unknown record tag 0x{b:02x}"),
            Self::UnknownYieldReason(b) => write!(f, "unknown yield reason 0x{b:02x}"),
            Self::UnknownHashKind(b) => write!(f, "unknown hash-checkpoint kind 0x{b:02x}"),
            Self::InvalidBool(b) => write!(f, "fault-discarded flag is neither 0 nor 1: 0x{b:02x}"),
            Self::UnknownEffectKind(b) => write!(f, "unknown effect kind 0x{b:02x}"),
            Self::UnknownBlockReason(b) => write!(f, "unknown block reason 0x{b:02x}"),
            Self::UnknownWakeReason(b) => write!(f, "unknown wake reason 0x{b:02x}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A single structured trace record.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TraceRecord {
    /// Scheduler selected a unit and granted it a budget.
    UnitScheduled {
        /// Unit that was scheduled.
        unit: UnitId,
        /// Budget granted to the unit for this step.
        granted_budget: Budget,
        /// Guest-time clock at scheduling.
        time: GuestTicks,
        /// Commit epoch at scheduling.
        epoch: Epoch,
    },
    /// A unit's `run_until_yield` returned.
    StepCompleted {
        /// Unit that completed the step.
        unit: UnitId,
        /// Reason the unit yielded.
        yield_reason: TracedYieldReason,
        /// Instruction cost consumed during the step.
        consumed_cost: InstructionCost,
        /// Guest-time clock after the step.
        time_after: GuestTicks,
    },
    /// Commit pipeline finished processing a step's effects.
    CommitApplied {
        /// Unit whose step was committed.
        unit: UnitId,
        /// Number of writes that landed in shared memory.
        writes_committed: u32,
        /// Number of effects deferred past this commit.
        effects_deferred: u32,
        /// Whether the step's effects were discarded due to a fault.
        fault_discarded: bool,
        /// Commit epoch after this commit.
        epoch_after: Epoch,
    },
    /// State hash captured at a controlled checkpoint.
    StateHashCheckpoint {
        /// Which subsystem the hash covers.
        kind: HashCheckpointKind,
        /// Hash value.
        hash: StateHash,
    },
    /// One record per effect, in emission order, with `sequence` running 0..N
    /// within the step. Effect payloads (write bytes, mailbox messages, DMA
    /// descriptors) are not included.
    EffectEmitted {
        /// Unit that emitted the effect.
        unit: UnitId,
        /// Per-step emission sequence number.
        sequence: u32,
        /// Effect variant.
        kind: TracedEffectKind,
    },
    /// Status overridden to `Blocked`. Emitted once per transition, after
    /// `CommitApplied`.
    UnitBlocked {
        /// Unit that transitioned to blocked.
        unit: UnitId,
        /// Why the unit blocked.
        reason: TracedBlockReason,
    },
    /// Status overridden to `Runnable` by the commit pipeline or a DMA
    /// completion. Emitted once per transition, after `CommitApplied`.
    UnitWoken {
        /// Unit that transitioned to runnable.
        unit: UnitId,
        /// Why the unit was woken.
        reason: TracedWakeReason,
    },
    /// Per-step PPU state fingerprint at instruction retire.
    ///
    /// `hash` covers GPR + LR + CTR + XER + CR under a canonical tooling-local
    /// byte layout. Emitted once per retired instruction when per-step tracing
    /// is active.
    PpuStateHash {
        /// Per-thread retired-instruction counter.
        step: u64,
        /// PC of the instruction that just retired.
        pc: u64,
        /// Hash of the PPU architectural state.
        hash: StateHash,
    },
    /// Full PPU register snapshot at instruction retire.
    ///
    /// Opt-in `[lo, hi]` window only, never on the hot path. Covers the same
    /// architectural surface as `PpuStateHash` uncompressed, so a divergence
    /// diff can name the exact disagreeing register. `step` matches
    /// `PpuStateHash::step` for the same instruction.
    PpuStateFull {
        /// Per-thread retired-instruction counter.
        step: u64,
        /// PC of the instruction that just retired.
        pc: u64,
        /// General-purpose registers r0..r31.
        gpr: [u64; 32],
        /// Link register.
        lr: u64,
        /// Count register.
        ctr: u64,
        /// Fixed-point exception register.
        xer: u64,
        /// Condition register.
        cr: u32,
    },
}

const TAG_UNIT_SCHEDULED: u8 = 0x00;
const TAG_STEP_COMPLETED: u8 = 0x01;
const TAG_COMMIT_APPLIED: u8 = 0x02;
const TAG_STATE_HASH_CHECKPOINT: u8 = 0x03;
const TAG_EFFECT_EMITTED: u8 = 0x04;
const TAG_UNIT_BLOCKED: u8 = 0x05;
const TAG_UNIT_WOKEN: u8 = 0x06;
const TAG_PPU_STATE_HASH: u8 = 0x07;
const TAG_PPU_STATE_FULL: u8 = 0x08;

impl TraceRecord {
    /// Trace level this record belongs to.
    pub fn level(&self) -> TraceLevel {
        match self {
            TraceRecord::UnitScheduled { .. }
            | TraceRecord::StepCompleted { .. }
            | TraceRecord::UnitBlocked { .. }
            | TraceRecord::UnitWoken { .. } => TraceLevel::Scheduling,
            TraceRecord::CommitApplied { .. } => TraceLevel::Commits,
            TraceRecord::StateHashCheckpoint { .. }
            | TraceRecord::PpuStateHash { .. }
            | TraceRecord::PpuStateFull { .. } => TraceLevel::Hashes,
            TraceRecord::EffectEmitted { .. } => TraceLevel::Effects,
        }
    }

    /// Append the binary encoding to `buf`.
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
                consumed_cost,
                time_after,
            } => {
                buf.push(TAG_STEP_COMPLETED);
                write_u64(buf, unit.raw());
                buf.push(*yield_reason as u8);
                write_u64(buf, consumed_cost.raw());
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
            TraceRecord::PpuStateHash { step, pc, hash } => {
                buf.push(TAG_PPU_STATE_HASH);
                write_u64(buf, *step);
                write_u64(buf, *pc);
                write_u64(buf, hash.raw());
            }
            TraceRecord::PpuStateFull {
                step,
                pc,
                gpr,
                lr,
                ctr,
                xer,
                cr,
            } => {
                buf.push(TAG_PPU_STATE_FULL);
                write_u64(buf, *step);
                write_u64(buf, *pc);
                for r in gpr.iter() {
                    write_u64(buf, *r);
                }
                write_u64(buf, *lr);
                write_u64(buf, *ctr);
                write_u64(buf, *xer);
                write_u32(buf, *cr);
            }
        }
    }

    /// Decode the next record from `bytes`, returning the record and bytes consumed.
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
                let consumed_cost = InstructionCost::new(read_u64(bytes, &mut pos)?);
                let time_after = GuestTicks::new(read_u64(bytes, &mut pos)?);
                TraceRecord::StepCompleted {
                    unit,
                    yield_reason,
                    consumed_cost,
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
            TAG_PPU_STATE_HASH => {
                let step = read_u64(bytes, &mut pos)?;
                let pc = read_u64(bytes, &mut pos)?;
                let hash = StateHash::new(read_u64(bytes, &mut pos)?);
                TraceRecord::PpuStateHash { step, pc, hash }
            }
            TAG_PPU_STATE_FULL => {
                let step = read_u64(bytes, &mut pos)?;
                let pc = read_u64(bytes, &mut pos)?;
                let mut gpr = [0u64; 32];
                for r in gpr.iter_mut() {
                    *r = read_u64(bytes, &mut pos)?;
                }
                let lr = read_u64(bytes, &mut pos)?;
                let ctr = read_u64(bytes, &mut pos)?;
                let xer = read_u64(bytes, &mut pos)?;
                let cr = read_u32(bytes, &mut pos)?;
                TraceRecord::PpuStateFull {
                    step,
                    pc,
                    gpr,
                    lr,
                    ctr,
                    xer,
                    cr,
                }
            }
            other => return Err(DecodeError::UnknownTag(other)),
        };
        Ok((record, pos))
    }
}

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
#[path = "tests/record_tests.rs"]
mod tests;
