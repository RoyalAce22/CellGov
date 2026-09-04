//! The versioned content store: where an installed firmware or title
//! version lives under the VFS root, and the record that describes it.
//!
//! [`layout`] is path arithmetic from an [`Artifact`] identity to a
//! directory; [`record`] is the schema of the TOML files those
//! directories are indexed by. The records are the index -- there is no
//! separate index file, so a reader enumerating installs scans
//! [`StoreLayout::installs_dir`]. [`pre_store`] refuses a root that
//! still holds the layout that came before this one.

pub mod layout;
pub mod pre_store;
pub mod record;

pub use layout::{
    record_rel_path, staging_sibling, tombstone_sibling, Artifact, ArtifactKind, StoreKeyError,
    StoreLayout, StorePathError, TitleId, TitleTree, VersionKey, DEFAULT_VFS_ROOT,
};
pub use pre_store::{preflight, PreStoreArtifact, PreStoreError, PreStoreResidue};
pub use record::{
    ArtifactRecord, InstallRecord, InstallRecordParseError, RapRecord, SourceRecord, TitleRecord,
    INSTALL_RECORD_FORMAT_VERSION,
};
