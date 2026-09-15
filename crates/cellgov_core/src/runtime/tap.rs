//! The debug observer a host installs on a runtime.

use cellgov_mem::GuestMemory;

/// What the runtime reports to the debug observer the host installs
/// with [`super::Runtime::set_tap`].
///
/// The runtime makes each report after it decides the fact, so an
/// observer cannot change what a run computes. Every method defaults
/// to a no-op.
pub trait RuntimeTap {
    /// `bytes` landed at guest address `addr`, in any address space.
    ///
    /// The runtime calls this once per staged write of a committed batch
    /// and once per host write, in the order that the writes land. It
    /// does not report these writes:
    ///
    /// - the boot's writes before the runtime exists;
    /// - the image that a [`super::ProcessSpawnLoader`] loads into a
    ///   child's address space;
    /// - the memory that [`super::Runtime::restore_into`] puts back.
    fn write(&mut self, _addr: u64, _bytes: &[u8]) {}

    /// Step number `step` ran; the first step is 1.
    ///
    /// `memory` is the boot address space before that step's batch
    /// commits. A read of a reserved region through `memory` logs no
    /// provisional read. A restore rewinds the step number and makes no
    /// report.
    fn step(&mut self, _step: u64, _memory: &GuestMemory) {}
}

#[cfg(test)]
#[path = "tests/tap_tests.rs"]
mod tests;
