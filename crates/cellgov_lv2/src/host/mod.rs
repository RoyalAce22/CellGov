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
mod config;
mod derived;
pub mod diagnostics;
mod dispatch_route;
mod event_flag;
mod event_queue;
mod expire;
mod fs;
pub(crate) mod guest_struct;
mod kernel_ids;
mod lv2_host;
mod lwcond;
mod lwmutex;
mod memory;
mod mmapper;
mod mutex;
mod observability;
mod ppu_thread;
mod privilege;
mod process;
mod raw_spu;
pub mod rsx;
mod runtime;
mod semaphore;
mod spu;
mod spu_event;
mod spu_group_control;
mod spu_local_store;
mod state;
mod state_hash;
mod system_ipc_witness;
mod uart;
mod usbd;

#[cfg(test)]
#[path = "tests/test_support.rs"]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "tests/host_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/hardware_trace_tests.rs"]
mod hardware_trace_tests;

#[cfg(test)]
#[path = "tests/sync_attribute_tests.rs"]
mod sync_attribute_tests;

#[cfg(test)]
#[path = "tests/error_code_console_tests.rs"]
mod error_code_console_tests;

#[cfg(test)]
#[path = "tests/process_queries_console_tests.rs"]
mod process_queries_console_tests;

#[cfg(test)]
#[path = "tests/process_classes_console_tests.rs"]
mod process_classes_console_tests;

#[cfg(test)]
#[path = "tests/kernel_id_classes_console_tests.rs"]
mod kernel_id_classes_console_tests;

#[cfg(test)]
#[path = "tests/timer_time_tty_console_tests.rs"]
mod timer_time_tty_console_tests;

#[cfg(test)]
#[path = "tests/timebase_console_tests.rs"]
mod timebase_console_tests;

#[cfg(test)]
#[path = "tests/arg_high_bits_console_tests.rs"]
mod arg_high_bits_console_tests;

#[cfg(test)]
#[path = "tests/usbd_config_hid_console_tests.rs"]
mod usbd_config_hid_console_tests;

#[cfg(test)]
#[path = "tests/config_service_records_console_tests.rs"]
mod config_service_records_console_tests;

#[cfg(test)]
#[path = "tests/config_listener_pad_data_console_tests.rs"]
mod config_listener_pad_data_console_tests;

#[cfg(test)]
#[path = "tests/uart_get_params_console_tests.rs"]
mod uart_get_params_console_tests;

pub use diagnostics::InvariantBreakReason;
pub use lv2_host::{FirmwareIdentity, Lv2Host};
pub use mmapper::SystemStateSeed;
pub use observability::{Lv2Observability, UnsupportedSyscallWitness};
pub use privilege::Privilege;
pub use process::{ProcessEntry, ProcessTable};
pub use rsx::{
    SysRsxContext, PACKAGE_CELLGOV_SET_FLIP_HANDLER, PACKAGE_CELLGOV_SET_USER_HANDLER,
    PACKAGE_CELLGOV_SET_VBLANK_HANDLER,
};
pub use runtime::Lv2Runtime;
pub use system_ipc_witness::{is_system_ipc_key, SystemIpcWitness};
