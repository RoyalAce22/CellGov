//! The SPU decrementer: a 32-bit count that runs down at the time-base
//! frequency, read through `SPU_RdDec` and loaded through `SPU_WrDec`.
//!
//! [CBEA p:139 s:9.7] each SPU has a 32-bit decrementer; a write to SPU_WrDec loads it and starts it.
//! [CBEA p:139 s:9.7] the SPU decrementers run at the rate of the PPE decrementer, the time base.
//!
//! The model keeps the count at one time-base instant and derives every
//! later value from the time base. The time base comes from the guest
//! clock through [`cellgov_time::ticks_to_tb`], the conversion the PPU's
//! `mftb` reads, so no host clock enters. The `spu_events` console
//! capture measures the rate: the decrementer and the time base agree to
//! one part in ten thousand over 140 million ticks.

use cellgov_time::{tb_to_ticks, ticks_to_tb, GuestTicks};

/// One SPU's decrementer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decrementer {
    /// The count at time base `since`.
    value: u32,
    /// The time base at which `value` held, or `None` before the unit
    /// first looks: the count starts at the unit's first instruction.
    since: Option<u64>,
    /// Whether the count runs.
    running: bool,
}

impl Decrementer {
    /// The decrementer of a new SPU thread: running from `u32::MAX`.
    ///
    /// The `spu_events` capture reads 0xFFFFF24D to 0xFFFFF77B at the
    /// thread's main entry, so the count runs down from the top of its
    /// range at thread start. Whether it starts at 0 or at `u32::MAX` is
    /// one tick the capture cannot tell apart. A start at 0 would cross
    /// zero at once and leave Tm pending, which the `spu_event_edges`
    /// capture does not show, so the model starts at `u32::MAX`.
    pub const fn new() -> Self {
        Self {
            value: u32::MAX,
            since: None,
            running: true,
        }
    }

    /// Whether the count runs.
    pub const fn running(&self) -> bool {
        self.running
    }

    /// Move the count to time base `now`. Returns whether the most
    /// significant bit changed from 0 to 1 on the way: the count passed
    /// zero.
    ///
    /// [CBEA p:139 s:9.7] a decrementer event occurs when the most significant bit changes from 0 to 1.
    ///
    /// As the count runs down, the bit turns from 0 to 1 only as it passes
    /// from 0 to `u32::MAX`, so the count crossed zero exactly when more
    /// ticks elapsed than it held.
    pub fn advance(&mut self, now: u64) -> bool {
        let since = *self.since.get_or_insert(now);
        self.since = Some(now);
        if !self.running {
            return false;
        }
        let elapsed = now.saturating_sub(since);
        let crossed = elapsed > u64::from(self.value);
        // Only the low 32 bits of the elapsed ticks move a 32-bit count.
        self.value = self.value.wrapping_sub(elapsed as u32);
        crossed
    }

    /// The count at time base `now`, and whether it crossed zero since
    /// the last look.
    ///
    /// [CBEA p:140 s:9.7.2] reading the decrementer does not change it.
    pub fn read(&mut self, now: u64) -> (u32, bool) {
        let crossed = self.advance(now);
        (self.value, crossed)
    }

    /// Load `value` at time base `now` and start the count. Returns
    /// whether a decrementer event occurs: the old count crossed zero
    /// before the write, or the write itself turns the most significant
    /// bit from 0 to 1.
    ///
    /// [CBEA p:139 s:9.7.1] a loaded value that changes the most significant bit from 0 to 1 signals the event at once; a load of 0 signals it after one decrementer interval.
    pub fn write(&mut self, now: u64, value: u32) -> bool {
        let crossed = self.advance(now);
        let msb = |v: u32| v & 0x8000_0000 != 0;
        let loaded = !msb(self.value) && msb(value);
        self.value = value;
        self.running = true;
        crossed || loaded
    }

    /// Stop the count at time base `now`. Returns whether it crossed zero
    /// before it stopped.
    ///
    /// [CBEA p:152 s:9.11.2] acknowledging the decrementer event while it is disabled stops the decrementer, and no further decrementer events occur.
    pub fn stop(&mut self, now: u64) -> bool {
        let crossed = self.advance(now);
        self.running = false;
        crossed
    }

    /// The first guest tick at which the running count passes zero, or
    /// `None` when the count does not run or has not started.
    pub fn crossing_tick(&self) -> Option<GuestTicks> {
        if !self.running {
            return None;
        }
        let tb = self
            .since?
            .saturating_add(u64::from(self.value))
            .saturating_add(1);
        let tick = tb_to_ticks(tb, 1);
        debug_assert!(tick == u64::MAX || ticks_to_tb(tick) >= tb);
        Some(GuestTicks::new(tick))
    }
}

impl Default for Decrementer {
    fn default() -> Self {
        Self::new()
    }
}

/// The time base at guest tick `now`.
pub fn time_base_at(now: GuestTicks) -> u64 {
    ticks_to_tb(now.raw())
}

#[cfg(test)]
#[path = "tests/decrementer_tests.rs"]
mod tests;
