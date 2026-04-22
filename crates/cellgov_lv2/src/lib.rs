//! cellgov_lv2 -- LV2 host model for managed SPU thread group lifecycle.
//!
//! This crate owns the LV2 concepts (image registry, thread group
//! table, request/response shapes, the dispatch function) and nothing
//! else. It does not depend on `cellgov_core`. The runtime owns
//! orchestration; this crate owns the state machine.
//!
//! Direction of the boundary: the runtime drives the host, the host
//! answers with pure data. The host never reaches into the runtime.
//! The only way the host reads guest memory is through the
//! `Lv2Runtime` trait the runtime implements.

pub mod dispatch;
pub mod errno;
pub mod host;
pub mod image;
pub mod ppu_thread;
pub mod request;
pub mod sync_primitives;
pub mod thread_group;

pub use dispatch::{
    CondMutexKind, Lv2BlockReason, Lv2Dispatch, PendingResponse, PpuThreadInitState,
    SpuImageHandle, SpuInitState,
};
pub use host::{Lv2Host, Lv2Runtime};
pub use image::{ContentStore, SpuImageRecord};
pub use ppu_thread::{
    AddJoinWaiter, EventFlagWaitMode, GuestBlockReason, PpuThread, PpuThreadAttrs, PpuThreadId,
    PpuThreadIdAllocator, PpuThreadState, PpuThreadTable, ThreadStack, ThreadStackAllocator,
    TlsTemplate,
};
pub use request::Lv2Request;
pub use sync_primitives::{
    CondEntry, CondTable, DuplicateEnqueue, EventFlagCreateError, EventFlagEnqueueError,
    EventFlagEntry, EventFlagTable, EventFlagWait, EventFlagWaiter, EventFlagWake, EventPayload,
    EventQueueEnqueueError, EventQueueEntry, EventQueueReceive, EventQueueSend, EventQueueTable,
    EventQueueWaiter, LwMutexAcquire, LwMutexAcquireOrEnqueue, LwMutexEnqueueError, LwMutexEntry,
    LwMutexIdAllocator, LwMutexRelease, LwMutexTable, MutexAcquire, MutexAcquireOrEnqueue,
    MutexAttrs, MutexCreateError, MutexEnqueueError, MutexEntry, MutexRelease, MutexTable,
    SemaphoreCreateError, SemaphoreEnqueueError, SemaphoreEntry, SemaphorePost, SemaphoreTable,
    SemaphoreWait, WaiterList,
};
pub use thread_group::{GroupState, ThreadGroup, ThreadGroupTable};
