//! The PS3 process boot. A title ELF and an installed firmware tree go
//! in; a runtime one `step()` from the title's first instruction comes
//! out. The step drivers carry that runtime to a terminal state.
//!
//! The crate owns image placement, firmware PRX loading and import
//! resolution, TLS and kernel-context setup, the `module_start` pass,
//! and the two step loops with their fault classifiers. It writes to no
//! console and ends no process: every refusal is a [`BootError`] and
//! every line of narration goes to a caller-supplied [`BootSink`].

#![cfg_attr(test, allow(clippy::unwrap_used))]

mod child_init;
mod content;
mod env;
mod error;
mod guest_args;
mod keys;
mod mounts;
mod prescan_format;
mod sink;
mod stack_walk;
mod taps;

pub mod diag;
pub mod manifest;
pub mod observation;
pub mod prepare;
pub mod prx;
pub mod step_loop;

pub use child_init::{ChildInitError, ChildInitPlans};
pub use content::{ContentBaseSource, ContentRegisterError};
pub use env::EnvBoolError;
pub use error::{BootError, NarrowError};
pub use guest_args::GuestArgsError;
pub use keys::KeyVaultSource;
pub use mounts::{ComposedMount, MountRegisterError};
pub use sink::{BootSink, NullSink};
pub use taps::{DebugTaps, NoTaps};
