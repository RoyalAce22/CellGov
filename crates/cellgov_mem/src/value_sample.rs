//! Env-gated per-step value sample on guest memory.
//!
//! Reads a fixed byte range at periodic CG-step boundaries. The
//! clock is step-gated, NOT commit-gated: the sample sees the
//! current region state regardless of which path wrote it, so a
//! byte that is zero at every sample was written non-zero by no
//! path at all. Complements [`super::store_watch`], which answers
//! "what writes address X via the commit pipeline."
//!
//! Env vars:
//!
//!   CELLGOV_VALUE_SAMPLE         `ADDR:WIDTH` in hex (e.g.
//!                                `0x91FE9C:4`); width in `[1, 256]`.
//!   CELLGOV_VALUE_SAMPLE_PATH    Output file path.
//!   CELLGOV_VALUE_SAMPLE_STRIDE  Optional decimal stride; default 1
//!                                (every step). 0 is rejected.
//!
//! File format (little-endian, no padding): a "CGVS" version-2
//! header (magic, version u32, addr u32, width u32), then one
//! record per sample: step u64, status u8 (0 = unmapped, 1 =
//! full-width read, 2 = short read), actual_len u32, value
//! bytes\[width\] zero-padded. `actual_len` exists so a short-read
//! padding zero stays distinguishable from a measured zero.

#![allow(clippy::print_stderr)]

use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

struct State {
    addr: u64,
    width: u32,
    stride: u64,
    writer: Mutex<BufWriter<File>>,
    samples_written: Mutex<u64>,
}

static STATE: OnceLock<Option<State>> = OnceLock::new();

/// Latch so a failing sink names itself once instead of per record.
static WRITE_FAILED: AtomicBool = AtomicBool::new(false);

fn init() -> Option<State> {
    let spec = env::var("CELLGOV_VALUE_SAMPLE").ok()?;
    let path = env::var("CELLGOV_VALUE_SAMPLE_PATH").ok()?;
    if spec.is_empty() || path.is_empty() {
        return None;
    }
    let stride: u64 = env::var("CELLGOV_VALUE_SAMPLE_STRIDE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    if stride == 0 {
        eprintln!("[cellgov] CELLGOV_VALUE_SAMPLE_STRIDE must be > 0");
        return None;
    }
    let (addr_s, width_s) = spec.split_once(':')?;
    let addr = u64::from_str_radix(addr_s.trim_start_matches("0x"), 16).ok()?;
    let width = u32::from_str_radix(width_s.trim_start_matches("0x"), 16).ok()?;
    if width == 0 || width > 256 {
        eprintln!("[cellgov] CELLGOV_VALUE_SAMPLE width out of range (1..=256): {width}");
        return None;
    }
    let file = File::create(&path).ok()?;
    let mut writer = BufWriter::new(file);
    writer.write_all(b"CGVS").ok()?;
    writer.write_all(&2u32.to_le_bytes()).ok()?;
    writer.write_all(&(addr as u32).to_le_bytes()).ok()?;
    writer.write_all(&width.to_le_bytes()).ok()?;
    writer.flush().ok()?;
    eprintln!(
        "[cellgov] value-sample active: addr=0x{addr:x} width={width} stride={stride} path={path}"
    );
    Some(State {
        addr,
        width,
        stride,
        writer: Mutex::new(writer),
        samples_written: Mutex::new(0),
    })
}

fn state() -> Option<&'static State> {
    STATE.get_or_init(init).as_ref()
}

/// Returns `Some((addr, width))` if the instrument is active and
/// `step` is a stride boundary. Caller reads the bytes via its own
/// guest-memory access path and passes them to [`emit`].
pub fn pending(step: u64) -> Option<(u64, u32)> {
    let s = state()?;
    if !step.is_multiple_of(s.stride) {
        return None;
    }
    Some((s.addr, s.width))
}

/// One record: `{ step u64, status u8, actual_len u32, value[width] }`,
/// little-endian.
///
/// Packed whole so one `write_all` either lands the record or none of
/// it: a reader walks fixed-size records from the header, and a torn
/// one shifts every later field.
fn pack_record(step: u64, status: u8, actual_len: u32, value: &[u8]) -> Vec<u8> {
    let mut record = Vec::with_capacity(13 + value.len());
    record.extend_from_slice(&step.to_le_bytes());
    record.push(status);
    record.extend_from_slice(&actual_len.to_le_bytes());
    record.extend_from_slice(value);
    record
}

/// Emit one sample record. `bytes` is `Some(slice)` when the read
/// returned anything (full or short), `None` when the address was
/// unmapped at this step. Short reads are recorded as status=2 with
/// `actual_len = bytes.len()`; the value tail past `actual_len` is
/// zero-padded.
pub fn emit(step: u64, bytes: Option<&[u8]>) {
    let Some(s) = state() else { return };
    let width = s.width as usize;
    let mut buf = vec![0u8; width];
    let (status, actual_len): (u8, u32) = match bytes {
        Some(b) if b.len() >= width => {
            buf.copy_from_slice(&b[..width]);
            (1, width as u32)
        }
        Some(b) => {
            buf[..b.len()].copy_from_slice(b);
            (2, b.len() as u32)
        }
        None => (0, 0),
    };
    let record = pack_record(step, status, actual_len, &buf);

    let mut writer = s.writer.lock().expect("value-sample writer mutex");
    // Per-record flush, matching `super::store_watch`: the writer lives
    // in a static that is never dropped, so a buffered tail would be
    // lost at process exit with nothing reporting the gap.
    if let Err(e) = writer.write_all(&record).and_then(|()| writer.flush()) {
        if !WRITE_FAILED.swap(true, Ordering::Relaxed) {
            eprintln!(
                "[cellgov] value-sample write failed at step {step}: {e};                  the capture is truncated from here"
            );
        }
        return;
    }
    // Counted only once the record is on disk: this is the operator's
    // completeness witness against the file's record count.
    let mut cnt = s
        .samples_written
        .lock()
        .expect("value-sample counter mutex");
    *cnt = cnt.wrapping_add(1);
}

/// Returns `Some(count)` if the instrument is active; `None` if env
/// vars are unset.
pub fn samples_written() -> Option<u64> {
    let s = state()?;
    Some(
        *s.samples_written
            .lock()
            .expect("value-sample counter mutex"),
    )
}

#[cfg(test)]
#[path = "tests/value_sample_tests.rs"]
mod tests;
