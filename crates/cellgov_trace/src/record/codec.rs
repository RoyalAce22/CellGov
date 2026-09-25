//! The tag bytes, the encoder and decoder, and the read and write primitives.

use crate::hash::StateHash;
use cellgov_event::UnitId;
use cellgov_time::{Budget, Epoch, GuestTicks, InstructionCost};

use super::error::DecodeError;
use super::reasons::{
    HashCheckpointKind, HostWriter, TracedBlockReason, TracedEffectKind,
    TracedInvariantBreakReason, TracedSyscallDisposition, TracedWakeReason, TracedYieldReason,
};
use super::trace_record::{TraceRecord, TRACE_FORMAT_VERSION};

pub(super) const TAG_UNIT_SCHEDULED: u8 = 0x00;
pub(super) const TAG_STEP_COMPLETED: u8 = 0x01;
pub(super) const TAG_COMMIT_APPLIED: u8 = 0x02;
pub(super) const TAG_STATE_HASH_CHECKPOINT: u8 = 0x03;
pub(super) const TAG_EFFECT_EMITTED: u8 = 0x04;
pub(super) const TAG_UNIT_BLOCKED: u8 = 0x05;
pub(super) const TAG_UNIT_WOKEN: u8 = 0x06;
pub(super) const TAG_PPU_STATE_HASH: u8 = 0x07;
pub(super) const TAG_PPU_STATE_FULL: u8 = 0x08;
pub(super) const TAG_HOST_INVARIANT_BREAK: u8 = 0x09;
pub(super) const TAG_SYSCALL_ENTERED: u8 = 0x0a;
pub(super) const TAG_RESERVED_REGION_READ: u8 = 0x0b;
pub(super) const TAG_SYSCALL_RETURNED: u8 = 0x0c;
pub(super) const TAG_RUN_IDENTITY: u8 = 0x0d;
pub(super) const TAG_HOST_WRITE: u8 = 0x0e;
pub(super) const TAG_STATE_HASH_SCHEME: u8 = 0x0f;

impl TraceRecord {
    /// Append the binary encoding to `buf`.
    pub fn encode(&self, buf: &mut Vec<u8>) {
        let start = buf.len();
        buf.push(self.tag());
        match self {
            TraceRecord::RunIdentity {
                format_version,
                firmware,
                game,
                overrides,
            } => {
                write_u32(buf, *format_version);
                write_u64(buf, *firmware);
                write_u64(buf, *game);
                write_u64(buf, *overrides);
            }
            TraceRecord::UnitScheduled {
                unit,
                granted_budget,
                time,
                epoch,
            } => {
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
                write_u64(buf, unit.raw());
                buf.push(u8::from(*yield_reason));
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
                write_u64(buf, unit.raw());
                write_u32(buf, *writes_committed);
                write_u32(buf, *effects_deferred);
                buf.push(if *fault_discarded { 1 } else { 0 });
                write_u64(buf, epoch_after.raw());
            }
            TraceRecord::StateHashCheckpoint { kind, hash } => {
                buf.push(u8::from(*kind));
                write_u64(buf, hash.raw());
            }
            TraceRecord::EffectEmitted {
                unit,
                sequence,
                kind,
            } => {
                write_u64(buf, unit.raw());
                write_u32(buf, *sequence);
                buf.push(u8::from(*kind));
            }
            TraceRecord::UnitBlocked { unit, reason } => {
                write_u64(buf, unit.raw());
                buf.push(u8::from(*reason));
            }
            TraceRecord::UnitWoken { unit, reason } => {
                write_u64(buf, unit.raw());
                buf.push(u8::from(*reason));
            }
            TraceRecord::PpuStateHash { step, pc, hash } => {
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
                reservation_line,
            } => {
                write_u64(buf, *step);
                write_u64(buf, *pc);
                for r in gpr.iter() {
                    write_u64(buf, *r);
                }
                write_u64(buf, *lr);
                write_u64(buf, *ctr);
                write_u64(buf, *xer);
                write_u32(buf, *cr);
                match reservation_line {
                    None => {
                        buf.push(0);
                        write_u64(buf, 0);
                    }
                    Some(addr) => {
                        buf.push(1);
                        write_u64(buf, *addr);
                    }
                }
            }
            TraceRecord::HostInvariantBreak { reason } => {
                buf.push(u8::from(*reason));
            }
            TraceRecord::SyscallEntered {
                unit,
                num,
                args,
                disposition,
            } => {
                write_u64(buf, unit.raw());
                write_u64(buf, *num);
                for a in args.iter() {
                    write_u64(buf, *a);
                }
                buf.push(u8::from(*disposition));
            }
            TraceRecord::ReservedRegionRead {
                unit,
                step,
                addr,
                len,
                hits,
            } => {
                write_u64(buf, unit.raw());
                write_u64(buf, *step);
                write_u64(buf, *addr);
                write_u32(buf, *len);
                write_u32(buf, *hits);
            }
            TraceRecord::SyscallReturned { unit, code, time } => {
                write_u64(buf, unit.raw());
                write_u64(buf, *code);
                write_u64(buf, time.raw());
            }
            TraceRecord::HostWrite {
                writer,
                space,
                addr,
                len,
                reservations_cleared,
            } => {
                buf.push(u8::from(*writer));
                write_u32(buf, *space);
                write_u64(buf, *addr);
                write_u32(buf, *len);
                write_u32(buf, *reservations_cleared);
            }
            TraceRecord::StateHashScheme { ppu, checkpoint } => {
                write_u64(buf, *ppu);
                write_u64(buf, *checkpoint);
            }
        }
        debug_assert_eq!(
            Some(buf.len() - start),
            Self::encoded_len(self.tag()),
            "encode wrote a different length than encoded_len declares for tag 0x{:02x}",
            self.tag()
        );
    }

    /// Decode the next record from `bytes`, returning the record and bytes consumed.
    ///
    /// # Errors
    ///
    /// [`DecodeError::UnknownTag`] for a tag no variant owns,
    /// [`DecodeError::UnsupportedFormatVersion`] for a header written
    /// under another format, [`DecodeError::Truncated`] when `bytes` is
    /// shorter than that tag's [`encoded_len`](Self::encoded_len), and
    /// the per-field variants when a byte inside the record is out of
    /// range.
    pub fn decode(bytes: &[u8]) -> Result<(Self, usize), DecodeError> {
        let mut pos = 0usize;
        let tag = read_u8(bytes, &mut pos)?;
        let Some(len) = Self::encoded_len(tag) else {
            return Err(DecodeError::UnknownTag(tag));
        };
        if tag == TAG_RUN_IDENTITY {
            // `len` is this format's header width, and a header of
            // another format has another width. Decode reads the version
            // first, so it reports an older, shorter header by its
            // format, and it reads no record at the wrong offset.
            let mut version_pos = pos;
            let found = read_u32(bytes, &mut version_pos)?;
            if found != TRACE_FORMAT_VERSION {
                return Err(DecodeError::UnsupportedFormatVersion(found));
            }
        }
        if bytes.len() < len {
            return Err(DecodeError::Truncated);
        }
        let record = match tag {
            TAG_RUN_IDENTITY => {
                let format_version = read_u32(bytes, &mut pos)?;
                let firmware = read_u64(bytes, &mut pos)?;
                let game = read_u64(bytes, &mut pos)?;
                let overrides = read_u64(bytes, &mut pos)?;
                TraceRecord::RunIdentity {
                    format_version,
                    firmware,
                    game,
                    overrides,
                }
            }
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
                let yield_reason = TracedYieldReason::try_from(yr_byte)?;
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
                let kind = HashCheckpointKind::try_from(kind_byte)?;
                let hash = StateHash::new(read_u64(bytes, &mut pos)?);
                TraceRecord::StateHashCheckpoint { kind, hash }
            }
            TAG_EFFECT_EMITTED => {
                let unit = UnitId::new(read_u64(bytes, &mut pos)?);
                let sequence = read_u32(bytes, &mut pos)?;
                let kind_byte = read_u8(bytes, &mut pos)?;
                let kind = TracedEffectKind::try_from(kind_byte)?;
                TraceRecord::EffectEmitted {
                    unit,
                    sequence,
                    kind,
                }
            }
            TAG_UNIT_BLOCKED => {
                let unit = UnitId::new(read_u64(bytes, &mut pos)?);
                let reason_byte = read_u8(bytes, &mut pos)?;
                let reason = TracedBlockReason::try_from(reason_byte)?;
                TraceRecord::UnitBlocked { unit, reason }
            }
            TAG_UNIT_WOKEN => {
                let unit = UnitId::new(read_u64(bytes, &mut pos)?);
                let reason_byte = read_u8(bytes, &mut pos)?;
                let reason = TracedWakeReason::try_from(reason_byte)?;
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
                let resv_tag = read_u8(bytes, &mut pos)?;
                let resv_addr = read_u64(bytes, &mut pos)?;
                let reservation_line = match resv_tag {
                    0 => None,
                    1 => Some(resv_addr),
                    other => return Err(DecodeError::InvalidBool(other)),
                };
                TraceRecord::PpuStateFull {
                    step,
                    pc,
                    gpr,
                    lr,
                    ctr,
                    xer,
                    cr,
                    reservation_line,
                }
            }
            TAG_HOST_INVARIANT_BREAK => {
                let reason_byte = read_u8(bytes, &mut pos)?;
                let reason = TracedInvariantBreakReason::try_from(reason_byte)?;
                TraceRecord::HostInvariantBreak { reason }
            }
            TAG_SYSCALL_ENTERED => {
                let unit = UnitId::new(read_u64(bytes, &mut pos)?);
                let num = read_u64(bytes, &mut pos)?;
                let mut args = [0u64; 8];
                for a in args.iter_mut() {
                    *a = read_u64(bytes, &mut pos)?;
                }
                let disposition_byte = read_u8(bytes, &mut pos)?;
                let disposition = TracedSyscallDisposition::try_from(disposition_byte)?;
                TraceRecord::SyscallEntered {
                    unit,
                    num,
                    args,
                    disposition,
                }
            }
            TAG_RESERVED_REGION_READ => {
                let unit = UnitId::new(read_u64(bytes, &mut pos)?);
                let step = read_u64(bytes, &mut pos)?;
                let addr = read_u64(bytes, &mut pos)?;
                let len = read_u32(bytes, &mut pos)?;
                let hits = read_u32(bytes, &mut pos)?;
                TraceRecord::ReservedRegionRead {
                    unit,
                    step,
                    addr,
                    len,
                    hits,
                }
            }
            TAG_SYSCALL_RETURNED => {
                let unit = UnitId::new(read_u64(bytes, &mut pos)?);
                let code = read_u64(bytes, &mut pos)?;
                let time = GuestTicks::new(read_u64(bytes, &mut pos)?);
                TraceRecord::SyscallReturned { unit, code, time }
            }
            TAG_HOST_WRITE => {
                let writer_byte = read_u8(bytes, &mut pos)?;
                let writer = HostWriter::try_from(writer_byte)?;
                let space = read_u32(bytes, &mut pos)?;
                let addr = read_u64(bytes, &mut pos)?;
                let len = read_u32(bytes, &mut pos)?;
                let reservations_cleared = read_u32(bytes, &mut pos)?;
                TraceRecord::HostWrite {
                    writer,
                    space,
                    addr,
                    len,
                    reservations_cleared,
                }
            }
            TAG_STATE_HASH_SCHEME => {
                let ppu = read_u64(bytes, &mut pos)?;
                let checkpoint = read_u64(bytes, &mut pos)?;
                TraceRecord::StateHashScheme { ppu, checkpoint }
            }
            other => return Err(DecodeError::UnknownTag(other)),
        };
        Ok((record, pos))
    }
}

pub(super) fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

pub(super) fn write_u64(buf: &mut Vec<u8>, v: u64) {
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
