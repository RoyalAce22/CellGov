//! Shared PRX-load types: [`PrxLoadInfo`] describes a loaded firmware
//! module to the rest of the crate; [`PrxLoadStageError`] names the
//! staging-time failure modes that [`super::got::patch_got_atomic`]
//! and [`super::load`] can surface.

/// One firmware module resident in guest memory.
pub struct PrxLoadInfo {
    /// Module name from its PRX header.
    pub name: String,
    /// Filesystem stem of the source PRX (e.g. `"libaudio"` for
    /// `libaudio.sprx`); empty when no source path is available.
    pub stem: String,
    /// Guest address the loader placed the module at.
    pub base: u64,
    /// Exclusive end of the loaded data segment. `alloc_base`
    /// must clear `max(data_end)` across all loaded PRXs or
    /// `sys_memory_allocate` hands out addresses inside a PRX.
    pub data_end: u64,
    /// The module's TOC.
    pub toc: u64,
    /// Relocations the loader applied.
    pub relocs_applied: usize,
    /// `module_start` descriptor, absent for a module that declares
    /// none.
    pub module_start: Option<cellgov_ppu::sprx::LoadedOpd>,
    /// `module_stop` descriptor, absent for a module that declares
    /// none.
    pub module_stop: Option<cellgov_ppu::sprx::LoadedOpd>,
}

impl PrxLoadInfo {
    /// True for the unresolved-import trampoline pseudo-module, which
    /// carries a guest region so the allocator clears it but has no
    /// firmware identity to register.
    ///
    /// `install_unresolved_trampolines_only` is the only constructor
    /// that produces this shape; a real module always names at least
    /// its stem.
    pub fn is_synthetic(&self) -> bool {
        self.module_start.is_none() && self.module_stop.is_none() && self.stem.is_empty()
    }
}

/// The two link-time maps the boot hands the LV2 host: the firmware
/// export view the sc 484 CoreOS manual link resolves against, and
/// the trampolined-NID requester list the unresolved-import
/// diagnostic names libraries from.
#[derive(Debug, Clone, Default)]
pub struct HostLinkMaps {
    /// Library name -> NID -> OPD, mirroring the loader's export table.
    pub exports: std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>>,
    /// Trampolined NID -> libraries whose import tables asked for it.
    pub unresolved_requesters: std::collections::BTreeMap<u32, std::collections::BTreeSet<String>>,
}

/// Firmware identity read from a `firmware.toml` whose entries
/// covered every PRX the boot loaded. Bound into the LV2 host so the
/// PUP revision folds into the state hash.
#[derive(Debug, Clone)]
pub struct VerifiedFirmware {
    /// `[firmware].image_version` from the manifest.
    pub image_version: String,
    /// `[firmware].pup_sha256` digest bytes.
    pub pup_sha256: [u8; 32],
}

/// Trampolined NID -> the library names whose import tables asked for
/// it, as the unresolved-import diagnostic reports them.
pub type UnresolvedRequesters = std::collections::BTreeMap<u32, std::collections::BTreeSet<String>>;

/// Why a firmware PRX failed to stage through the GOT-patch path.
#[derive(Debug, thiserror::Error)]
pub enum PrxLoadStageError {
    /// The module file did not read.
    #[error("read {}: {source}", path.display())]
    Read {
        /// The module file.
        path: std::path::PathBuf,
        /// What the read refused with.
        #[source]
        source: std::io::Error,
    },
    /// The operator's key vault did not load, so an SCE-wrapped module
    /// cannot be opened.
    #[error("key vault for {}: {detail}", path.display())]
    KeyVault {
        /// The module file the vault was needed for.
        path: std::path::PathBuf,
        /// The vault loader's own account of the refusal.
        detail: String,
    },
    /// The module's SCE wrapper did not decrypt.
    #[error("decrypt {}: {source}", path.display())]
    Decrypt {
        /// The module file.
        path: std::path::PathBuf,
        /// The decrypter's own account of the refusal.
        #[source]
        source: cellgov_install::sce::SceError,
    },
    /// A GOT slot address is not a writable 4-byte range.
    #[error("GOT slot at 0x{stub_addr:08x} (nid 0x{nid:08x}): invalid 4-byte range")]
    GotSlotBadRange {
        /// Guest address of the stub that holds the slot.
        stub_addr: u32,
        /// The NID the slot binds.
        nid: u32,
    },
    /// The value destined for a 4-byte GOT slot (a resolved OPD or a
    /// trampoline slot) lies beyond the 32-bit guest address space;
    /// truncating it would alias an unrelated guest address.
    #[error(
        "GOT slot for nid 0x{nid:08x}: value 0x{addr:016x} exceeds the 32-bit guest address space"
    )]
    GotSlotValueBeyondU32 {
        /// The NID the slot binds.
        nid: u32,
        /// The value that did not fit.
        addr: u64,
    },
    /// `StagingMemory::drain_into` rejected the batch; guest memory
    /// is unchanged by the atomic-batch contract.
    #[error("GOT batch validation failed ({source}); {staged} staged write(s) discarded")]
    GotBatchCommit {
        /// Writes the batch held at the refusal.
        staged: usize,
        /// What the commit pipeline refused with.
        #[source]
        source: cellgov_mem::MemError,
    },
}
