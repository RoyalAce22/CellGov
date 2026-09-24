//! The counters a finished run reports as anomalies, and which of them
//! makes the run's own result suspect.

use cellgov_core::Runtime;

/// Counters the runtime and the step loop kept that a run reports as
/// anomalies.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RunAnomalies {
    /// Reads answered zero from a reserved RSX or SPU region.
    pub provisional_reads: u64,
    /// Pending wake responses overwritten before the guest drained them.
    pub response_displacements: usize,
    /// `sys_tty_write` calls whose buffer left mapped memory.
    pub tty_oob_dropped: usize,
    /// `sys_tty_write` calls whose fd did not fit in `u32`.
    pub tty_bogus_fd: usize,
}

impl RunAnomalies {
    /// Read the runtime's counters after the loop, beside the two the
    /// step loop kept itself ([`crate::step_loop::StepLoopCtx`]'s
    /// `tty_oob_count` and `bogus_fd_count`).
    #[must_use]
    pub fn read(rt: &Runtime, tty_oob_dropped: usize, tty_bogus_fd: usize) -> Self {
        Self {
            provisional_reads: rt.memory().provisional_read_count(),
            response_displacements: rt.syscall_responses().displacement_count(),
            tty_oob_dropped,
            tty_bogus_fd,
        }
    }

    /// Whether a counter makes the run's own result suspect.
    ///
    /// A displaced response is the one such counter: the guest lost an
    /// `r3` and its out-pointer writes. The others name work the run
    /// dropped.
    #[must_use]
    pub fn had_critical_anomaly(&self) -> bool {
        self.response_displacements > 0
    }
}

#[cfg(test)]
#[path = "tests/anomalies_tests.rs"]
mod tests;
