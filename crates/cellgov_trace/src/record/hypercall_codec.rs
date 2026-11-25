//! The encoding of the hypercall records: entry, return and console.

use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

use super::codec::{
    read_bool, read_u64, read_u8, write_u64, TAG_HYPERCALL_ENTERED, TAG_HYPERCALL_RETURNED,
};
use super::error::DecodeError;
use super::reasons::TracedHypercallDisposition;
use super::trace_record::{TraceRecord, CONSOLE_RECORD_BYTES};

/// Appends the payload of a hypercall record to `buf`.
///
/// The caller writes the tag first. Any other record appends nothing.
pub(super) fn encode(record: &TraceRecord, buf: &mut Vec<u8>) {
    match record {
        TraceRecord::HypercallEntered {
            unit,
            number,
            args,
            pc,
            thread,
            disposition,
            time,
        } => {
            write_u64(buf, unit.raw());
            write_u64(buf, *number);
            for a in args {
                write_u64(buf, *a);
            }
            write_u64(buf, *pc);
            buf.push(u8::from(thread.is_some()));
            buf.push(thread.unwrap_or(0));
            buf.push(u8::from(*disposition));
            write_u64(buf, time.raw());
        }
        TraceRecord::HypercallReturned {
            unit,
            status,
            outputs,
            written,
            untimed,
            time,
        } => {
            write_u64(buf, unit.raw());
            write_u64(buf, *status);
            for o in outputs {
                write_u64(buf, *o);
            }
            buf.push(*written);
            buf.push(u8::from(*untimed));
            write_u64(buf, time.raw());
        }
        TraceRecord::HypervisorConsole {
            unit,
            console,
            len,
            bytes,
        } => {
            write_u64(buf, unit.raw());
            write_u64(buf, *console);
            buf.push(*len);
            buf.extend_from_slice(bytes);
        }
        _ => {}
    }
}

/// Decodes the payload of the hypercall record `tag` names, from `pos`.
///
/// A tag other than the entry and return tags decodes as a console
/// record.
///
/// # Errors
///
/// - [`DecodeError::Truncated`] when `bytes` ends inside the payload.
/// - [`DecodeError::InvalidBool`] when a flag byte is not 0 or 1.
/// - [`DecodeError::UnknownHypercallDisposition`] when the disposition
///   byte names no variant.
/// - [`DecodeError::CountOutOfRange`] when `written` or `len` counts
///   more entries than its array holds.
pub(super) fn decode(tag: u8, bytes: &[u8], pos: &mut usize) -> Result<TraceRecord, DecodeError> {
    let unit = UnitId::new(read_u64(bytes, pos)?);
    if tag == TAG_HYPERCALL_ENTERED {
        let number = read_u64(bytes, pos)?;
        let mut args = [0u64; 8];
        for a in args.iter_mut() {
            *a = read_u64(bytes, pos)?;
        }
        let pc = read_u64(bytes, pos)?;
        let has_thread = read_bool(bytes, pos)?;
        let thread = read_u8(bytes, pos)?;
        let disposition = TracedHypercallDisposition::try_from(read_u8(bytes, pos)?)?;
        let time = GuestTicks::new(read_u64(bytes, pos)?);
        return Ok(TraceRecord::HypercallEntered {
            unit,
            number,
            args,
            pc,
            thread: has_thread.then_some(thread),
            disposition,
            time,
        });
    }
    if tag == TAG_HYPERCALL_RETURNED {
        let status = read_u64(bytes, pos)?;
        let mut outputs = [0u64; 7];
        for o in outputs.iter_mut() {
            *o = read_u64(bytes, pos)?;
        }
        let written = read_u8(bytes, pos)?;
        if usize::from(written) > outputs.len() {
            return Err(DecodeError::CountOutOfRange(written));
        }
        let untimed = read_bool(bytes, pos)?;
        let time = GuestTicks::new(read_u64(bytes, pos)?);
        return Ok(TraceRecord::HypercallReturned {
            unit,
            status,
            outputs,
            written,
            untimed,
            time,
        });
    }
    let console = read_u64(bytes, pos)?;
    let len = read_u8(bytes, pos)?;
    if usize::from(len) > CONSOLE_RECORD_BYTES {
        return Err(DecodeError::CountOutOfRange(len));
    }
    let mut data = [0u8; CONSOLE_RECORD_BYTES];
    for b in data.iter_mut() {
        *b = read_u8(bytes, pos)?;
    }
    Ok(TraceRecord::HypervisorConsole {
        unit,
        console,
        len,
        bytes: data,
    })
}
