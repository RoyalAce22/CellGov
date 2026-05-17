//! LV2 model the runtime calls into.
//!
//! # Cross-module contract
//!
//! The runtime calls [`Lv2Host::dispatch`] once per PPU syscall yield,
//! synchronously inside the same `step()` that observed the yield.
//! During the call the host reads guest memory through the
//! [`Lv2Runtime`] trait and returns an [`crate::dispatch::Lv2Dispatch`] telling the
//! runtime what guest-visible work to perform. The host never writes
//! guest memory directly; every write travels back to the runtime as
//! an `Effect` so the commit pipeline orders it.

mod callback_dispatch;
mod cond;
mod diagnostics;
mod dispatch_route;
mod event_flag;
mod event_queue;
mod fs;
mod lv2_host;
mod lwmutex;
mod memory;
mod mutex;
mod ppu_thread;
mod process;
pub mod rsx;
mod runtime;
mod semaphore;
mod spu;
mod state_hash;

#[cfg(test)]
mod test_support;

#[cfg(test)]
#[path = "../tests/host_tests.rs"]
mod cross_primitive_tests;

pub use callback_dispatch::CallbackError;
pub use lv2_host::{FirmwareIdentity, Lv2Host};
pub use rsx::{
    SysRsxContext, PACKAGE_CELLGOV_SET_FLIP_HANDLER, PACKAGE_CELLGOV_SET_USER_HANDLER,
    PACKAGE_CELLGOV_SET_VBLANK_HANDLER,
};
pub use runtime::Lv2Runtime;
