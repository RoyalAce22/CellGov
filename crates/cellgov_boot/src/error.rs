//! The one error a boot refusal reaches its caller as.

use crate::child_init::ChildInitError;
use crate::prepare::{
    EntryError, HostBindError, ImageError, ParamsError, PatchError, ProviderError,
    StrictReservedConflict,
};
use crate::prx::{FirmwareLoadError, ModuleStartError, TlsError};

/// Why a boot did not reach the title's first instruction.
///
/// One variant per stage of [`crate::prepare`], plus the two failures
/// the step loop can raise once the boot runs: a child's init pass and
/// a `module_start` the loop drove.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    /// The guest address space or the title image in it.
    #[error("{0}")]
    Image(#[from] ImageError),

    /// The process parameters read out of the title ELF.
    #[error("{0}")]
    Params(#[from] ParamsError),

    /// The firmware PRX set, or the imports bound against it.
    #[error("{0}")]
    Firmware(#[from] FirmwareLoadError),

    /// TLS pre-init or the kernel-context OPD.
    #[error("{0}")]
    Tls(#[from] TlsError),

    /// The runtime and the identity bound into its LV2 host.
    #[error("{0}")]
    Host(#[from] HostBindError),

    /// The primary thread's entry state.
    #[error("{0}")]
    Entry(#[from] EntryError),

    /// Guest-visible content, images or mounts.
    #[error("{0}")]
    Providers(#[from] ProviderError),

    /// A `--patch-byte` write after the `module_start` pass.
    #[error("{0}")]
    Patch(#[from] PatchError),

    /// A `module_start` that left its unit mid-execution.
    #[error("{0}")]
    ModuleStart(#[from] ModuleStartError),

    /// A spawned child's init pass.
    #[error("{0}")]
    ChildInit(#[from] ChildInitError),

    /// Two boot options that cannot both hold.
    #[error("{0}")]
    StrictReserved(#[from] StrictReservedConflict),

    /// A boot-computed value too wide for the guest ABI field it goes in.
    #[error("{0}")]
    Narrow(#[from] NarrowError),
}

/// A boot-computed address or size that does not fit the `u32` the
/// guest ABI carries it in.
///
/// Truncating would alias an unrelated guest address, so the boot stops
/// instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{label}: 0x{value:016x} does not fit in u32")]
pub struct NarrowError {
    /// What the boot computed the value for.
    pub label: &'static str,
    /// The value that did not fit.
    pub value: u64,
}

/// Narrow a boot-computed address or size to the `u32` the guest ABI
/// carries it in.
///
/// # Errors
///
/// [`NarrowError`] when `value` exceeds `u32::MAX`.
pub(crate) fn narrow_u32(label: &'static str, value: u64) -> Result<u32, NarrowError> {
    u32::try_from(value).map_err(|_| NarrowError { label, value })
}

#[cfg(test)]
#[path = "tests/error_tests.rs"]
mod tests;
