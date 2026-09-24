//! The versioned content store: where an installed firmware or title
//! version lives under the VFS root, and the record that describes it.
//!
//! [`layout`] is path arithmetic from an [`Artifact`] identity to a
//! directory; [`record`] is the schema of the TOML files those
//! directories are indexed by. The records are the index -- there is no
//! separate index file, so a reader enumerating installs scans
//! [`StoreLayout::installs_dir`], and [`inventory`] is that scan.
//! [`pre_store`] refuses a root that
//! still holds the layout that came before this one. [`verify`] holds
//! a tree against the record that describes it. [`lock`] keeps two
//! writers off one entry; readers take nothing. [`rename`] is the one
//! policy every publish and teardown rename goes through.

pub mod inventory;
pub mod layout;
pub mod lock;
pub mod pre_store;
pub mod record;
pub mod rename;
pub mod verify;

pub use inventory::{
    dir_exists, BaseEntry, FirmwareEntry, InventoryError, StoreInventory, TitleEntry, UpdateEntry,
};
pub use layout::{
    components_under, hdd0_exdata_dir, record_rel_path, staging_sibling, tombstone_sibling,
    Artifact, ArtifactKind, HiddenSiblingError, StoreKeyError, StoreLayout, StorePathError,
    TitleId, TitleTree, VersionKey, BASE_GAME_VER, CORE_OS_DIR, DEFAULT_VFS_ROOT,
};
pub use lock::{lock_artifact, lock_firmware_staging, StoreLock, StoreLockError};
pub use pre_store::{preflight, PreStoreArtifact, PreStoreError, PreStoreResidue};
pub use record::{
    ArtifactRecord, CoreOsFileRecord, CoreOsRecord, InstallRecord, InstallRecordParseError,
    KernelRecord, RapRecord, SourceRecord, TitleRecord, DISC_DISTRIBUTION,
    INSTALL_RECORD_FORMAT_VERSION, PSN_HDD_DISTRIBUTION,
};
pub use rename::{rename_with_retry, RenameRefused};
pub use verify::{verify_record_tree, Divergence, DivergenceKind, VerifyReadError, VerifyReport};
