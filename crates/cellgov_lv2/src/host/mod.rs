//! LV2 model the runtime calls into.
//!
//! # Cross-module contract
//!
//! The runtime calls [`Lv2Host::dispatch`] once per PPU syscall yield,
//! synchronously inside the same `step()` that observed the yield.
//! The host reads guest memory through [`Lv2Runtime`] and returns an
//! [`crate::dispatch::Lv2Dispatch`]; every guest-visible write travels
//! back to the runtime as an `Effect` so the commit pipeline orders it.

mod cond;
pub mod diagnostics;
mod dispatch_route;
mod event_flag;
mod event_queue;
mod fs;
mod lv2_host;
mod lwmutex;
mod memory;
mod mmapper;
mod mutex;
mod ppu_thread;
mod process;
pub mod rsx;
mod runtime;
mod semaphore;
mod spu;
mod state_hash;

#[cfg(test)]
#[path = "tests/test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "tests/host_tests.rs"]
mod tests;

pub use diagnostics::InvariantBreakReason;
pub use lv2_host::{FirmwareIdentity, Lv2Host};
pub use mmapper::SystemStateSeed;
pub use rsx::{
    SysRsxContext, PACKAGE_CELLGOV_SET_FLIP_HANDLER, PACKAGE_CELLGOV_SET_USER_HANDLER,
    PACKAGE_CELLGOV_SET_VBLANK_HANDLER,
};
pub use runtime::Lv2Runtime;
