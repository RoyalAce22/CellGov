//! Typed LV2 syscall requests decoded from PPU `sc` GPR state.
//!
//! [`classify`] is total: unknown numbers and malformed arguments
//! surface as [`Lv2Request::Unsupported`] / [`Lv2Request::Malformed`]
//! rather than panicking, so host dispatch can match exhaustively.
//!
//! # Cross-crate contract
//!
//! Pointer fields are guest effective addresses (u32, big-endian on
//! the bus). The classifier rejects a u32-typed slot whose source
//! GPR has non-zero high 32 bits rather than truncating; downstream
//! `Lv2Host` handlers therefore never receive a silently-narrowed
//! pointer. Out-pointers carry the convention that the kernel must
//! commit `*out = id` before returning OK -- the runtime emits the
//! write and the OK return as a single atomic effect batch so guests
//! that race a sibling thread on the id never observe a stale slot.

mod classify;
mod types;

pub use classify::{classify, classify_with_lev};
pub use types::Lv2Request;
