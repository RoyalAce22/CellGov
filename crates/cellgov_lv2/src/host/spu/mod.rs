//! SPU-lifecycle LV2 dispatch: image open, thread-group
//! create/start/initialize/join, mailbox and signal writes, and the
//! signal configuration.

mod channel;
mod group;
mod image;
mod thread;

pub(in crate::host) use group::GroupCreateArgs;

#[cfg(test)]
#[path = "tests/spu_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/spu_signal_console_tests.rs"]
mod signal_console_tests;

#[cfg(test)]
#[path = "tests/spu_user_image_tests.rs"]
mod user_image_tests;

#[cfg(test)]
#[path = "tests/spu_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/spu_init_limits_console_tests.rs"]
mod init_limits_console_tests;

#[cfg(test)]
#[path = "tests/spu_pool_tests.rs"]
mod pool_tests;

#[cfg(test)]
#[path = "tests/spu_pool_console_tests.rs"]
mod pool_console_tests;

#[cfg(test)]
#[path = "tests/spu_pool_edges_console_tests.rs"]
mod pool_edges_console_tests;

#[cfg(test)]
#[path = "tests/spu_group_container_console_tests.rs"]
mod group_container_console_tests;

#[cfg(test)]
#[path = "tests/spu_group_root_types_console_tests.rs"]
mod root_types_console_tests;

#[cfg(test)]
#[path = "tests/spu_thread_argument_tests.rs"]
mod thread_argument_tests;
