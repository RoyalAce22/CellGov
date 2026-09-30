//! An MFC exception the host has not taken: a queued command the MFC
//! refused, which suspended its SPU's command queue.
//!
//! The MFC reports the refusal to the PPE as an interrupt, which LV2
//! turns into the SPU thread-group exception event. The interrupt class
//! depends on the refusal:
//!
//! - class 0 for a refused command or parameter;
//! - class 1 for an address that does not translate.
//!
//! CellGov keeps the first such exception for the host, which ends the
//! run on it.
//!
//! [CBEA p:263 s:21.4 Table 21-3] the DMA alignment and invalid DMA command interrupts are class 0; the MFC data-segment and data-storage interrupts are class 1.

use cellgov_dma::{InvalidMfcCommand, RaisedMfcCommand};
use cellgov_event::UnitId;

use super::Runtime;

/// A command the MFC refused, and the SPU it suspended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MfcException {
    /// The SPU unit whose queue suspended.
    pub unit: UnitId,
    /// The SPU thread group holding the unit, if one does.
    pub group: Option<u32>,
    /// The command and the check it failed.
    pub command: InvalidMfcCommand,
}

impl core::fmt::Display for MfcException {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let class = match self.command.error.class() {
            cellgov_dma::MfcExceptionClass::Alignment => "DMA alignment",
            cellgov_dma::MfcExceptionClass::InvalidCommand => "invalid DMA command",
            cellgov_dma::MfcExceptionClass::DataSegment => "MFC data segment",
            cellgov_dma::MfcExceptionClass::DataStorage => "MFC data storage",
        };
        let p = self.command.params;
        write!(f, "SPU unit {}", self.unit.raw())?;
        if let Some(group) = self.group {
            write!(f, " (group 0x{group:08x})")?;
        }
        write!(
            f,
            ": {class}: {} (cmd 0x{:08x} lsa 0x{:08x} eah 0x{:08x} eal 0x{:08x} size 0x{:08x} tag 0x{:08x})",
            self.command.error, self.command.word, p.lsa, p.eah, p.eal, p.size, p.tag
        )
    }
}

impl Runtime {
    /// The first MFC exception since the last take, which the host acts
    /// on after a commit.
    pub fn take_mfc_exception(&mut self) -> Option<MfcException> {
        self.mfc_exception.take()
    }

    /// The run's MFC exception as a drain would leave it: the one the
    /// queue raised, or else the first one it would raise if it drained
    /// now. Nothing in the runtime changes.
    ///
    /// A run whose units have all finished stops with commands still
    /// queued, and a refused one among them is still the run's
    /// exception. A caller that must not land the queued transfers --
    /// one comparing the run's state -- reads it here instead of
    /// draining. Translation depends on the region layout alone, which a
    /// completion never changes, so the answer is the drain's.
    pub fn mfc_exception_at_drain(&self) -> Option<MfcException> {
        if self.mfc_exception.is_some() {
            return self.mfc_exception;
        }
        let memory = &self.memory;
        let mut queue = self.dma_queue.clone();
        let due = queue
            .process_due_translating(cellgov_time::GuestTicks::new(u64::MAX), |c, payloaded| {
                super::dma::translation_fault(memory, c, payloaded)
            });
        due.raised.first().map(|raised| MfcException {
            unit: raised.issuer,
            group: self.lv2_host.live_spu_group(raised.issuer),
            command: raised.command,
        })
    }

    /// Record a command the queue raised. The first one stands until the
    /// host takes it.
    pub(super) fn record_mfc_exception(&mut self, raised: RaisedMfcCommand) {
        let group = self.lv2_host.live_spu_group(raised.issuer);
        self.mfc_exception.get_or_insert(MfcException {
            unit: raised.issuer,
            group,
            command: raised.command,
        });
    }
}

#[cfg(test)]
#[path = "tests/mfc_exception_at_drain_tests.rs"]
mod mfc_exception_at_drain_tests;
