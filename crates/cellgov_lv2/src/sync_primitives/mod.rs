//! Per-primitive state tables and the shared FIFO waiter list.
//!
//! Wake order is strictly FIFO by enqueue order.

pub mod cond;
mod errors;
pub mod event_flag;
pub mod event_queue;
pub mod lwmutex;
pub mod mutex;
pub mod semaphore;
mod waiter_list;

pub use cond::{CondCreateError, CondEnqueueError, CondEntry, CondSignalToError, CondTable};
pub use errors::{DuplicateEnqueue, IdCollision};
pub use event_flag::{
    EventFlagCreateError, EventFlagEnqueueError, EventFlagEntry, EventFlagTable, EventFlagWait,
    EventFlagWaiter, EventFlagWake,
};
pub use event_queue::{
    EventPayload, EventQueueEnqueueError, EventQueueEntry, EventQueueReceive, EventQueueSend,
    EventQueueTable, EventQueueWaiter,
};
pub use lwmutex::{
    LwMutexAcquire, LwMutexAcquireOrEnqueue, LwMutexEnqueueError, LwMutexEntry, LwMutexIdAllocator,
    LwMutexRelease, LwMutexTable,
};
pub use mutex::{
    MutexAcquire, MutexAcquireOrEnqueue, MutexAttrs, MutexCreateError, MutexEnqueueError,
    MutexEntry, MutexRelease, MutexTable,
};
pub use semaphore::{
    SemaphoreCreateError, SemaphoreEnqueueError, SemaphoreEntry, SemaphorePost, SemaphorePostN,
    SemaphoreTable, SemaphoreWait,
};
pub use waiter_list::WaiterList;
