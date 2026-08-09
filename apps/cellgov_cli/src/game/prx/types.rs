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

/// The two link-time maps the boot hands the LV2 host: the firmware
/// export view the sc 484 CoreOS manual link resolves against, and
/// the trampolined-NID requester list the unresolved-import
/// diagnostic names libraries from.
#[derive(Debug, Clone, Default)]
pub(in crate::game) struct HostLinkMaps {
    /// Library name -> NID -> OPD, mirroring the loader's export table.
    pub exports: std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>>,
    /// Trampolined NID -> libraries whose import tables asked for it.
    pub unresolved_requesters: std::collections::BTreeMap<u32, std::collections::BTreeSet<String>>,
}

/// Firmware identity read from a `firmware.toml` whose entries
/// covered every PRX the boot loaded. Bound into the LV2 host so the
/// PUP revision folds into the state hash.
#[derive(Debug, Clone)]
pub(in crate::game) struct VerifiedFirmware {
    /// `[firmware].image_version` from the manifest.
    pub image_version: String,
    /// `[firmware].pup_sha256` digest bytes.
    pub pup_sha256: [u8; 32],
}

/// Why a firmware PRX failed to stage through the GOT-patch path.
#[derive(Debug, thiserror::Error)]
pub(super) enum PrxLoadStageError {
    #[error("read {}: {source}", path.display())]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("decrypt {}: {source}", path.display())]
    Decrypt {
        path: std::path::PathBuf,
        #[source]
        source: cellgov_install::sce::SceError,
    },
    #[error("GOT slot at 0x{stub_addr:08x} (nid 0x{nid:08x}): invalid 4-byte range")]
    GotSlotBadRange { stub_addr: u32, nid: u32 },
    /// The value destined for a 4-byte GOT slot (a resolved OPD or a
    /// trampoline slot) lies beyond the 32-bit guest address space;
    /// truncating it would alias an unrelated guest address.
    #[error(
        "GOT slot for nid 0x{nid:08x}: value 0x{addr:016x} exceeds the 32-bit guest address space"
    )]
    GotSlotValueBeyondU32 { nid: u32, addr: u64 },
    /// `StagingMemory::drain_into` rejected the batch; guest memory
    /// is unchanged by the atomic-batch contract.
    #[error("GOT batch validation failed ({source}); {staged} staged write(s) discarded")]
    GotBatchCommit {
        staged: usize,
        #[source]
        source: cellgov_mem::MemError,
    },
}
