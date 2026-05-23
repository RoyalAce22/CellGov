//! [`ExecuteVerdict`] -- outcome of a single
//! [`super::execute`](super::execute) call.

use crate::exec::fault::PpuFault;

/// Outcome of a single `execute` call.
// [PPC-Book1 p:5 s:1.5] non-branching insns set NIA=CIA+4; branches assign NIA explicitly.
#[derive(Debug, PartialEq, Eq)]
pub enum ExecuteVerdict {
    /// Advance PC by 4.
    Continue,
    /// PC was written explicitly; caller must not advance.
    Branch,
    /// Yield to runtime syscall dispatch.
    // [PPC-Book3 p:12 s:2.3.1] sc with LEV=1 invokes the hypervisor; LEV>1 reserved.
    // [PPC-Book1 p:26 s:2.4.2] sc LEV-field encoding (instruction bits 20:25 reserved).
    Syscall {
        /// LEV field of `sc`: 0 = kernel syscall, 1 = hypercall,
        /// greater than 1 reserved.
        lev: u8,
    },
    /// Architectural fault.
    Fault(PpuFault),
    /// Memory access into an unmapped region. Carries the
    /// `cellgov_mem::MemError` produced at the loader boundary; the
    /// effective address is reachable via
    /// `MemError::Unmapped(FaultContext).addr`.
    MemFault(cellgov_mem::MemError),
    /// Store buffer full; caller flushes, yields, then retries the
    /// same instruction.
    BufferFull,
}
