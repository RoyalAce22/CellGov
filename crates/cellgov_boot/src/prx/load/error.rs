//! Why a firmware set did not load, or did not bind.

use std::path::PathBuf;

use crate::prx::types::PrxLoadStageError;

/// Why a firmware set did not load, or did not bind.
#[derive(Debug, thiserror::Error)]
pub enum FirmwareLoadError {
    /// The walk could not read a directory under the firmware root.
    #[error("prx: read_dir {}: {source}", dir.display())]
    ReadDir {
        /// The directory the walk could not read.
        dir: PathBuf,
        /// What the walk refused with.
        #[source]
        source: std::io::Error,
    },
    /// One entry of a firmware directory could not be read.
    #[error("prx: read_dir entry under {}: {source}", dir.display())]
    ReadDirEntry {
        /// The directory the entry sits in.
        dir: PathBuf,
        /// What the read refused with.
        #[source]
        source: std::io::Error,
    },
    /// No `firmware.toml` covers the directory the boot names.
    #[error(
        "no firmware.toml at or above {}; the installed firmware is unverifiable. \
         Reinstall with `cellgov firmware install`, which writes the manifest.",
        dir.display()
    )]
    NoManifest {
        /// The directory the walk started from.
        dir: PathBuf,
    },
    /// The manifest could not be read.
    #[error("read {}: {source}", path.display())]
    ManifestRead {
        /// The manifest the walk found.
        path: PathBuf,
        /// What the read refused with.
        #[source]
        source: std::io::Error,
    },
    /// The manifest could not be parsed.
    #[error("{}: {source}", path.display())]
    ManifestParse {
        /// The manifest the walk found.
        path: PathBuf,
        /// The parser's own account of the refusal.
        #[source]
        source: Box<cellgov_install::manifest::ManifestError>,
    },
    /// A loaded module sits outside the root its manifest governs.
    #[error(
        "firmware module {} is outside the manifest root {}; the manifest-root walk and \
         the module path disagree",
        file.display(),
        root.display()
    )]
    ModuleOutsideRoot {
        /// The module the walk selected.
        file: PathBuf,
        /// The install root its manifest governs.
        root: PathBuf,
    },
    /// A loaded module has no `[[files]]` entry vouching for it.
    #[error(
                "{}: not listed in firmware.toml ({rel:?}); the installed firmware and its manifest disagree. \
         Reinstall with `cellgov firmware install`.",
        file.display()
    )]
    ModuleNotInManifest {
        /// The module with no entry.
        file: PathBuf,
        /// Its root-relative path, as the manifest would spell it.
        rel: String,
    },
    /// A loaded module's post-decrypt bytes do not match the manifest.
    #[error(
        "{}: post-decrypt SHA-256 mismatch against firmware.toml\n  \
         expected {expected}\n  actual   {actual}\n\
         The file does not match the installed PUP revision.",
        file.display()
    )]
    ModuleDigestMismatch {
        /// The module that did not match.
        file: PathBuf,
        /// The digest the manifest records, in hex.
        expected: String,
        /// The digest its post-decrypt bytes hash to, in hex.
        actual: String,
    },
    /// A module file could not be read or decrypted.
    #[error("prx: {0}")]
    Stage(#[from] PrxLoadStageError),
    /// The title's own import tables do not parse, so no namespace
    /// names a firmware module to select.
    #[error("imports: parse failed: {source}")]
    ImportParse {
        /// The parser's own account of the refusal.
        source: cellgov_ppu::prx::ImportParseError,
    },
    /// Rounding the guest heap floor up to a 64 KB page overflowed.
    #[error("alloc_floor=0x{alloc_floor:x} + 0xFFFF overflows usize")]
    AllocFloorOverflow {
        /// The floor the round-up started from.
        alloc_floor: usize,
    },
    /// A firmware path is not Unicode, so it cannot key the candidate
    /// set.
    #[error("prx: non-utf8 firmware path: {}", path.display())]
    NonUtf8Path {
        /// The path as the host spells it.
        path: PathBuf,
    },
    /// The firmware directory holds no module to select from.
    #[error("prx: firmware-set mode: no .sprx modules under {}", dir.display())]
    NoModules {
        /// The directory that held none.
        dir: PathBuf,
    },
    /// A `sys/internal` stem a firmware executable cannot boot without
    /// is absent.
    #[error("prx: firmware-exec boot needs sys/internal/{stem}, absent under {}", dir.display())]
    InternalStemAbsent {
        /// The stem the shell loads by path.
        stem: &'static str,
        /// The directory the stem walk covered.
        dir: PathBuf,
    },
    /// A `sys/internal` stem is present but import-closure selection
    /// dropped it, leaving the shell's load-by-path unbacked.
    #[error(
        "prx: firmware-exec boot needs sys/internal/{stem}, but selection dropped it: {reason}"
    )]
    InternalStemDropped {
        /// The stem the shell loads by path.
        stem: &'static str,
        /// Why selection dropped it, or that selection did not choose it.
        reason: String,
    },
    /// Import-closure selection refused the candidate set.
    #[error("prx: firmware-set selection failed: {source}")]
    Selection {
        /// The selector's own account of the refusal.
        #[source]
        source: cellgov_ppu::prx_loader::PrxLoaderError,
    },
    /// A selected module's path carries no usable filename stem.
    #[error("prx: cannot derive a module stem from {path}")]
    NoStem {
        /// The selected path with no filename stem.
        path: String,
    },
    /// A selected module failed to parse as a PRX.
    #[error("prx: failed to parse {path}: {source}")]
    ParseModule {
        /// The module that did not parse.
        path: String,
        /// The parser's own account of the refusal.
        source: cellgov_ppu::sprx::PrxParseError,
    },
    /// The `prx_base` boot override names no placement the main region
    /// can hold.
    #[error("--prx-base 0x{base:016x}: {reason}")]
    PrxBase {
        /// The base the override named.
        base: u64,
        /// Which rule it broke.
        reason: String,
    },
    /// Rounding the PRX placement base up to a page overflowed.
    #[error("page_align_up_u64: 0x{addr:016x} + 0xFFF overflows")]
    PageAlignOverflow {
        /// The address the round-up started from.
        addr: u64,
    },
    /// The loader could not place the selected set.
    #[error("prx: firmware-set load failed at base 0x{base:016x}: {source}")]
    LoadSet {
        /// The placement base the loader was given.
        base: u64,
        /// The loader's own account of the refusal.
        source: cellgov_ppu::prx_loader::PrxLoaderError,
    },
    /// The resident boot image would overlap a fixed boot reservation.
    #[error(
        "boot image and firmware set end at 0x{image_end:016x}, past the TLS reservation at \
         0x{tls_base:016x}"
    )]
    RegionSize {
        /// Highest address exclusive of the title image and firmware set.
        image_end: u64,
        /// First address of the fixed TLS reservation.
        tls_base: u64,
    },
    /// The GOT batch was rejected, so no import was bound.
    #[error("prx: firmware-set GOT patch aborted ({source})")]
    GotPatch {
        /// Which staging step refused the batch.
        #[source]
        source: PrxLoadStageError,
    },
    /// The trampoline-only GOT batch was rejected, so every import slot
    /// still holds its pre-load bytes.
    #[error("prx: trampoline-only GOT patch aborted ({source})")]
    TrampolineGotPatch {
        /// Which staging step refused the batch.
        #[source]
        source: PrxLoadStageError,
    },
    /// The loader's export table answered `keys()` with a pair its
    /// `get()` does not hold.
    #[error("prx: export table key {namespace:?}::0x{nid:08x} vanished between keys() and get()")]
    ExportVanished {
        /// The export namespace the key named.
        namespace: String,
        /// The NID the key named.
        nid: u32,
    },
    /// An export OPD lies outside the 32-bit guest address space.
    #[error(
        "prx: export {namespace:?}::0x{nid:08x} OPD 0x{opd:016x} exceeds the 32-bit \
         guest address space"
    )]
    ExportBeyondU32 {
        /// The export namespace.
        namespace: String,
        /// The NID.
        nid: u32,
        /// The OPD address that did not fit.
        opd: u64,
    },
    /// The loader's topological order names a module absent from its
    /// loaded set.
    #[error(
        "prx: topological order names module id 0x{id:08x} absent from the loaded set; \
         the loader's order/loaded invariant broke"
    )]
    OrderWithoutModule {
        /// The module id the order named.
        id: u32,
    },
    /// A loaded module has no recorded filesystem stem.
    #[error(
        "prx: loaded module id 0x{id:08x} ({name:?}) has no recorded stem; the loader's \
         module-id/stem invariant broke"
    )]
    ModuleWithoutStem {
        /// The loaded module's id.
        id: u32,
        /// Its name from its PRX header.
        name: String,
    },
}
