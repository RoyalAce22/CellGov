//! cellgov_lv2 -- LV2 host model for managed SPU thread group lifecycle.
//!
//! This crate owns the LV2 concepts (image registry, thread group
//! table, request/response shapes, the dispatch function) and nothing
//! else. It does not depend on `cellgov_core`. The runtime owns
//! orchestration; this crate owns the state machine.
//!
//! The direction of the seam: the runtime drives the host, the host
//! answers with pure data. The host never reaches into the runtime.
//! The only way the host reads guest memory is through the
//! `Lv2Runtime` trait the runtime implements.

pub mod dispatch;
pub mod host;
pub mod image;
pub mod request;
pub mod thread_group;

pub use dispatch::{Lv2BlockReason, Lv2Dispatch, PendingResponse, SpuImageHandle, SpuInitState};
pub use host::{Lv2Host, Lv2Runtime};
pub use image::{ContentStore, SpuImageRecord};
pub use request::Lv2Request;
pub use thread_group::{GroupState, ThreadGroup, ThreadGroupTable};
