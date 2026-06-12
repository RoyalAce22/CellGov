//! LV2 host model: image registry, thread group table, sync primitives, and syscall dispatch.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod dispatch;
pub mod fs_store;
pub mod host;
pub mod image;
pub mod ppu_thread;
pub mod prx_registry;
pub mod request;
pub mod sync_primitives;
pub mod syscall_classification;
pub mod thread_group;

pub use dispatch::{
    CondMutexKind, Lv2BlockReason, Lv2Dispatch, PendingResponse, PpuThreadInitState, SpuInitState,
};
pub use fs_store::{FileStat, FsError, FsMount, FsMountTable, FsStore, SeekWhence};
pub use host::{InvariantBreakReason, Lv2Host, Lv2Runtime, SystemStateSeed};
pub use image::{ContentStore, SpuImageHandle, SpuImageRecord};
pub use ppu_thread::{
    AddJoinWaiter, EventFlagWaitMode, GuestBlockReason, PpuThread, PpuThreadAttrs, PpuThreadId,
    PpuThreadIdAllocator, PpuThreadState, PpuThreadTable, ThreadStack, ThreadStackAllocator,
    TlsTemplate,
};
pub use prx_registry::{LoadedPrxEntry, LoadedPrxRegistry};
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
pub use syscall_classification::{classify as classify_syscall, SyscallClassification};
pub use thread_group::{GroupState, ThreadGroup, ThreadGroupTable};
