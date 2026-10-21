//! The MFC transfer times a retail console measures, as a latency model.
//!
//! [`MeasuredMfcLatency`] charges a transfer an MFC command issued the
//! time the `spu_dma_timing` capture measured for its direction and size
//! (`cellgov_ps3_abi::hw::mfc_timing`), converted from the time base to
//! guest ticks. Between two measured sizes the time is linear in the
//! size. A transfer with either address off a 128-byte line takes the
//! table measured 16 bytes past a line. A transfer no MFC command issued
//! keeps the fixed latency of [`FixedLatency`].
//!
//! The `spu_dma_contention` capture shows how transfers share the bus.
//! A transfer's time is a latency and its bytes. The latency of
//! transfers queued back to back under one tag overlaps, but each one's
//! bytes follow the bytes of its MFC's transfers before it; the model
//! applies the same order across tags, which no capture times. Beside
//! SPUs streaming gets, every SPU with a transfer in flight takes an
//! equal share of [`MFC_SHARED_BYTES_PER_TB`], and the model shares
//! puts the same way. A transfer behind another holds the queue no less
//! than [`MFC_TRANSFER_MIN_TB_SUM`]. The share is fixed when a transfer
//! issues, from the SPUs with bytes in flight then.

use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::mfc_timing::{
    MFC_GET_LINE_TB_SUM, MFC_GET_OFF_LINE_TB_SUM, MFC_PUT_LINE_TB_SUM, MFC_PUT_OFF_LINE_TB_SUM,
    MFC_SHARED_BYTES_PER_TB, MFC_TAG_WAIT_TB_SUM, MFC_TIMING_LINE_BYTES, MFC_TIMING_RUNS,
    MFC_TIMING_SIZES, MFC_TRANSFER_MIN_TB_SUM,
};
use cellgov_time::{tb_to_ticks, GuestTicks};

use crate::latency::{DmaLatencyModel, FixedLatency};
use crate::queue::DmaQueue;
use crate::request::{DmaDirection, DmaRequest};

/// Latency from the console's measured MFC transfer times.
pub struct MeasuredMfcLatency {
    fallback: FixedLatency,
}

impl MeasuredMfcLatency {
    /// The model, with `fallback_ticks` for a transfer no MFC command
    /// issued.
    pub const fn new(fallback_ticks: u64) -> Self {
        Self {
            fallback: FixedLatency::new(fallback_ticks),
        }
    }
}

/// The transfer's own time in time-base ticks, summed over the
/// capture's runs: the table entry less the tag wait it holds.
fn transfer_tb_sum(direction: DmaDirection, off_line: bool, size: u64) -> u64 {
    let table = match (direction, off_line) {
        (DmaDirection::Get, false) => &MFC_GET_LINE_TB_SUM,
        (DmaDirection::Get, true) => &MFC_GET_OFF_LINE_TB_SUM,
        (DmaDirection::Put, false) => &MFC_PUT_LINE_TB_SUM,
        (DmaDirection::Put, true) => &MFC_PUT_OFF_LINE_TB_SUM,
    };
    let own = |i: usize| u64::from(table[i].saturating_sub(MFC_TAG_WAIT_TB_SUM));
    let sizes = MFC_TIMING_SIZES.map(u64::from);
    let last = sizes.len() - 1;
    if size <= sizes[0] {
        return own(0);
    }
    if size >= sizes[last] {
        return own(last);
    }
    // The measured size at or below `size`; the next one is above it.
    let i = sizes.iter().rposition(|&s| s <= size).unwrap_or(0);
    let (lo, hi) = (sizes[i], sizes[i + 1]);
    let (t_lo, t_hi) = (own(i), own(i + 1));
    // Linear between the two points; either may be the larger, since
    // the small sizes sit inside the runs' spread.
    let span = hi - lo;
    let into = size - lo;
    if t_hi >= t_lo {
        t_lo + ((t_hi - t_lo) * into).div_ceil(span)
    } else {
        t_lo - ((t_lo - t_hi) * into) / span
    }
}

/// The guest ticks the console's MFC takes for a transfer of `size`
/// bytes in `direction`, either address `off_line` or not.
pub fn measured_transfer_ticks(direction: DmaDirection, off_line: bool, size: u64) -> u64 {
    let sum = transfer_tb_sum(direction, off_line, size);
    tb_to_ticks(sum, u64::from(MFC_TIMING_RUNS)).max(1)
}

/// The time `size` bytes take at one SPU's own rate in `direction`,
/// summed over the capture's runs: the slope between the two largest
/// sizes of the on-line table.
fn own_bytes_tb_sum(direction: DmaDirection, size: u64) -> u64 {
    let table = match direction {
        DmaDirection::Get => &MFC_GET_LINE_TB_SUM,
        DmaDirection::Put => &MFC_PUT_LINE_TB_SUM,
    };
    let last = MFC_TIMING_SIZES.len() - 1;
    let span = u64::from(MFC_TIMING_SIZES[last] - MFC_TIMING_SIZES[last - 1]);
    let rise = u64::from(table[last] - table[last - 1]);
    (size * rise).div_ceil(span)
}

/// The time `size` bytes take with `sharers` SPUs on the bus, summed
/// over the capture's runs.
fn bytes_tb_sum(direction: DmaDirection, size: u64, sharers: u64) -> u64 {
    let shared = (size * sharers * u64::from(MFC_TIMING_RUNS)).div_ceil(MFC_SHARED_BYTES_PER_TB);
    own_bytes_tb_sum(direction, size).max(shared)
}

/// The SPUs with a transfer in flight at `now`: `issuer` and every other
/// issuer with a queued MFC transfer of bytes that completes after
/// `now`.
fn sharers(queued: &DmaQueue, issuer: UnitId, now: GuestTicks) -> u64 {
    let mut others: Vec<UnitId> = queued
        .pending()
        .map(|(c, _)| c)
        .filter(|c| c.issuer() != issuer && c.completion_time() > now)
        .filter(|c| c.request().length() > 0 && c.request().command_word().is_some())
        .map(|c| c.issuer())
        .collect();
    others.sort_unstable();
    others.dedup();
    1 + others.len() as u64
}

impl DmaLatencyModel for MeasuredMfcLatency {
    fn completion_time(&self, req: &DmaRequest, now: GuestTicks, queued: &DmaQueue) -> GuestTicks {
        if req.command_word().is_none() {
            return self.fallback.completion_time(req, now, queued);
        }
        let off_line = !req
            .source()
            .start()
            .raw()
            .is_multiple_of(MFC_TIMING_LINE_BYTES)
            || !req
                .destination()
                .start()
                .raw()
                .is_multiple_of(MFC_TIMING_LINE_BYTES);
        let (direction, size) = (req.direction(), req.length());
        let runs = u64::from(MFC_TIMING_RUNS);
        let alone = transfer_tb_sum(direction, off_line, size);
        let latency = alone.saturating_sub(own_bytes_tb_sum(direction, size));
        let bytes = bytes_tb_sum(direction, size, sharers(queued, req.issuer(), now));
        let start = now.saturating_add(GuestTicks::new(tb_to_ticks(latency + bytes, runs).max(1)));
        // A command that moves no bytes queues behind nothing here; the
        // ordering floor holds a barrier behind what it names.
        if size == 0 {
            return start;
        }
        // Behind its MFC's earlier transfers, a transfer's bytes follow
        // theirs and hold the queue no less than the least a transfer does.
        let queued_bytes = bytes.max(MFC_TRANSFER_MIN_TB_SUM);
        let behind = queued
            .issuer_latest_completion(req.issuer())
            .saturating_add(GuestTicks::new(tb_to_ticks(queued_bytes, runs).max(1)));
        start.max(behind)
    }

    fn invalid_command_time(&self, now: GuestTicks, queued: &DmaQueue) -> GuestTicks {
        self.fallback.invalid_command_time(now, queued)
    }
}

#[cfg(test)]
#[path = "tests/measured_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/measured_sharing_tests.rs"]
mod sharing_tests;
