//! Per-syscall dispatch helpers for the `Lv2Request::Unsupported
//! { number: N }` arms: one submodule per syscall family.

mod memory;
mod misc;
mod prx;
mod thread;
