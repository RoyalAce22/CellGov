//! LV2 host model: image registry, thread group table, sync primitives, and syscall dispatch.

pub mod dispatch;
pub mod fs;
pub mod host;
pub mod image;
pub mod ppu_thread;
pub mod request;
pub mod sync_primitives;
pub mod syscall_classification;
pub mod thread_group;

pub use dispatch::{
    CallbackReturnStage, CondMutexKind, Lv2BlockReason, Lv2Dispatch, PendingResponse,
    PpuThreadInitState, SpuImageHandle, SpuInitState,
};
pub use fs::{FileStat, FsError, FsStore, SeekWhence};
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
pub use syscall_classification::{classify as classify_syscall, SyscallClassification};
pub use thread_group::{GroupState, ThreadGroup, ThreadGroupTable};
