//! Shared PRX-load types: [`PrxLoadInfo`] describes a loaded firmware
//! module to the rest of `game/`; [`PrxLoadStageError`] names the
//! staging-time failure modes that [`super::got::patch_got_atomic`]
//! and [`super::load`] can surface.

pub(in crate::game) struct PrxLoadInfo {
    pub(in crate::game) name: String,
    /// Filesystem stem of the source PRX (e.g. `"libaudio"` for
    /// `libaudio.sprx`); empty when no source path is available.
    pub(in crate::game) stem: String,
    pub(in crate::game) base: u64,
    /// Exclusive end of the loaded data segment. `alloc_base`
    /// must clear `max(data_end)` across all loaded PRXs or
    /// `sys_memory_allocate` hands out addresses inside a PRX.
    pub(in crate::game) data_end: u64,
    pub(in crate::game) toc: u64,
    pub(in crate::game) relocs_applied: usize,
    pub(in crate::game) module_start: Option<cellgov_ppu::sprx::LoadedOpd>,
    pub(in crate::game) module_stop: Option<cellgov_ppu::sprx::LoadedOpd>,
}

/// Why a firmware PRX failed to stage through the GOT-patch path.
#[derive(Debug, thiserror::Error)]
pub(super) enum PrxLoadStageError {
    /// Reading the firmware module file failed.
    #[error("read {}: {source}", path.display())]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// SCE / SELF decryption failed.
    #[error("decrypt {}: {source}", path.display())]
    Decrypt {
        path: std::path::PathBuf,
        #[source]
        source: cellgov_firmware::sce::SceError,
    },
    /// A staged GOT slot's 4-byte ByteRange could not be constructed.
    #[error("GOT slot at 0x{stub_addr:08x} (nid 0x{nid:08x}): invalid 4-byte range")]
    GotSlotBadRange { stub_addr: u32, nid: u32 },
    /// `StagingMemory::drain_into` rejected the batch; guest memory
    /// is unchanged by the atomic-batch contract.
    #[error("GOT batch validation failed ({source}); {staged} staged write(s) discarded")]
    GotBatchCommit {
        staged: usize,
        #[source]
        source: cellgov_mem::MemError,
    },
}
