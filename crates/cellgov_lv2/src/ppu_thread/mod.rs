//! PPU thread table owned by the LV2 host.
//!
//! The runtime never touches this directly; the scheduler only
//! observes `UnitStatus::Blocked` and the guest-semantic reason
//! lives here so the host can transition the unit back to
//! runnable when the underlying condition resolves.

mod block_reason;
mod id;
mod stack;
mod table;
mod thread;
mod tls;

pub use block_reason::{EventFlagWaitMode, GuestBlockReason};
pub use id::{PpuThreadId, PpuThreadIdAllocator};
pub use stack::{ThreadStack, ThreadStackAllocator};
pub use table::PpuThreadTable;
pub use thread::{AddJoinWaiter, PpuThread, PpuThreadAttrs, PpuThreadState};
pub use tls::TlsTemplate;
