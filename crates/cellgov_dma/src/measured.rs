//! The MFC transfer times a retail console measures, as a latency model.
//!
//! [`MeasuredMfcLatency`] charges a transfer an MFC command issued the
//! time the `spu_dma_timing` capture measured for its direction and size
//! (`cellgov_ps3_abi::hw::mfc_timing`), converted from the time base to
//! guest ticks. Between two measured sizes the time is linear in the
//! size. A transfer with either address off a 128-byte line takes the
//! table measured 16 bytes past a line. A transfer no MFC command issued
//! keeps the fixed latency of [`FixedLatency`].

use cellgov_ps3_abi::hw::mfc_timing::{
    MFC_GET_LINE_TB_SUM, MFC_GET_OFF_LINE_TB_SUM, MFC_PUT_LINE_TB_SUM, MFC_PUT_OFF_LINE_TB_SUM,
    MFC_TAG_WAIT_TB_SUM, MFC_TIMING_LINE_BYTES, MFC_TIMING_RUNS, MFC_TIMING_SIZES,
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
        let ticks = measured_transfer_ticks(req.direction(), off_line, req.length());
        now.saturating_add(GuestTicks::new(ticks))
    }

    fn invalid_command_time(&self, now: GuestTicks, queued: &DmaQueue) -> GuestTicks {
        self.fallback.invalid_command_time(now, queued)
    }
}

#[cfg(test)]
#[path = "tests/measured_tests.rs"]
mod tests;
