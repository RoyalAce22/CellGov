//! LV2 kernel ABI: errno values, syscall numbers and the namespace
//! layout over them, and each subsystem's flag bits, struct layouts
//! and object ids.

pub mod config;
pub mod errno;
pub mod fs;
pub mod ipc;
pub mod memory;
pub mod namespace;
pub mod ppu_thread;
pub mod process;
pub mod prx;
pub mod rsx;
pub mod spu;
pub mod ss;
pub mod sync;
pub mod syscall;
pub mod uart;
pub mod usbd;
