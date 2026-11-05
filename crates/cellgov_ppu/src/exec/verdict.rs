//! Outcome of a single [`super::execute`] call.

use crate::exec::fault::PpuFault;

/// Outcome of a single `execute` call.
///
/// [PPC-Book1 p:5 s:1.5] non-branching insns set NIA=CIA+4; branches assign NIA explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecuteVerdict {
    /// Advance PC by 4.
    Continue,
    /// PC was written explicitly; caller must not advance.
    Branch,
    /// Yield to runtime syscall dispatch.
    ///
    /// [PPC-Book3 p:12 s:2.3.1] sc with LEV=1 invokes the hypervisor; LEV>1 reserved.
    /// [PPC-Book1 p:26 s:2.4.2] sc LEV-field encoding (instruction bits 20:25 reserved).
    Syscall {
        /// LEV of `sc`. Always 0: `sc` with LEV = 1 executes to
        /// `PpuInterrupt::SystemCall`.
        lev: u8,
    },
    /// Architectural fault.
    Fault(PpuFault),
    /// Memory access into an unmapped region. Carries the
    /// `cellgov_mem::MemError` produced at the loader boundary; the
    /// effective address is reachable via
    /// `MemError::Unmapped(FaultContext).addr`.
    MemFault(cellgov_mem::MemError),
    /// A store, `dcbz` or conditional store to a byte no region holds,
    /// or that a region refuses writes to. The error's address is the
    /// first refused byte.
    ///
    /// [CBE-Handbook p:249 s:9.5.3] A data storage exception occurs when a byte a store names cannot be translated, and the interrupt is precise.
    StoreFault(cellgov_mem::MemError),
    /// Store buffer full; caller flushes, yields, then retries the
    /// same instruction.
    BufferFull,
}

impl ExecuteVerdict {
    /// Whether the verdict permits a store-with-update RA writeback or
    /// fused store-pair second-half execution. Only `Continue` does.
    ///
    /// Exhaustive: a new variant must explicitly declare its policy or
    /// risk corrupting RA on deferred-success / failure paths.
    pub fn allows_writeback(&self) -> bool {
        match self {
            ExecuteVerdict::Continue => true,
            ExecuteVerdict::Branch
            | ExecuteVerdict::Syscall { .. }
            | ExecuteVerdict::Fault(_)
            | ExecuteVerdict::MemFault(_)
            | ExecuteVerdict::StoreFault(_)
            | ExecuteVerdict::BufferFull => false,
        }
    }
}
