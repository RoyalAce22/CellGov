//! Per-syscall dispatch helpers for the `Lv2Request::Unsupported
//! { number: N }` arms: one submodule per syscall family, plus the
//! readers they share in [`be`].

mod be;
mod memory;
mod misc;
mod prx;
mod thread;
