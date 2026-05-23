//! In-memory filesystem layer.
//!
//! [`FsStore`] is host-side data populated at boot from a per-title
//! content manifest and consumed by the `sys_fs_*` LV2 dispatch
//! handlers. It is the back end for guest reads from paths a title
//! expects to load (`/app_home/Data/.../*.xml`, configuration files,
//! etc.); cellgov has no real filesystem so the bytes come from the
//! title manifest's `[content]` block.
//!
//! A blob is registered once at boot and is immutable thereafter.
//! Each `sys_fs_open` allocates a fresh fd whose offset advances
//! independently per fd; two opens of the same path each see the
//! file from byte 0.
//!
//! All ids are deterministic: fds come from a monotonic counter that
//! never recycles within a boot.

mod error;
mod mount;
mod store;

pub use error::FsError;
pub use mount::{FsMount, FsMountTable};
pub use store::{DirEntry, FileStat, FsStore, SeekWhence};
