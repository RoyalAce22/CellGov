//! CellGov's hypervisor answers the LV1 hypercalls that a thread in
//! supervisor state makes. It answers them in lv1's place, and does not
//! run or emulate lv1.
//!
//! Each hypercall group registers a handler for each number it answers,
//! in the group's own module. A handler is a pure function of these
//! inputs:
//! - the [`HypercallRequest`];
//! - the firmware version the [`Hypervisor`] answers for;
//! - the [`GroupStates`] the [`Hypervisor`] holds;
//! - the caller's real address space, which holds the page table.
//!
//! It returns a [`HypercallDisposition`]:
//! - an answer, which the runtime applies as effects:
//!   - r3;
//!   - r4 to r10, where lv1's return loads them;
//!   - r0, r11, r12 and CR, as lv1's return leaves them;
//!   - memory writes;
//!   - the page table registers of the caller;
//! - a typed refusal, which stops the run.
//!
//! No handler writes guest state. An answer carries a list of
//! [`StateChange`]s, each a change to one group's state, and
//! [`Hypervisor`] applies them.
//!
//! [`answer()`] gives the order of the answers: lv1's own `sc` vector,
//! then the handler, then the null backend. [`Hypervisor`] keeps a
//! witness of each number with no handler.
//!
//! [`Hypervisor`] also adds to each answer the external interrupt of each
//! return that the call causes. It tests a thread's take of a mediated
//! external interrupt as lv1's handler tests it.

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::disallowed_macros,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]

mod answer;
mod cost;
mod external;
mod group;
mod handlers;
mod identity;
mod interprocessor;
mod mediated;
mod request;
mod return_path;
mod state;
mod vector;

pub use answer::{
    ConsoleWrite, HypercallAnswer, HypercallDisposition, HypercallRefusal, OutputRegisters,
    PageTableLoad, ReturnPath, ThreadInterrupt, ThreadResume,
};
pub use cost::cost;
pub use group::device_bus::{
    DeviceBusChange, DeviceBusState, DmaMapping, DmaRegion, OpenDevice, StorageBusOpen,
    StorageDeviceOpen, StorageDevicePort,
};
pub use group::gpu::{GpuChange, GpuState};
pub use group::interrupt::bitmap::{
    IrqStateBitmapKey, IrqStateBitmapRegistration, IrqStateBitmapState, StatusShadowWrite,
    SHADOW_DOUBLEWORDS,
};
pub use group::interrupt::event_port::{EventReceivePort, EventReceivePortState};
pub use group::interrupt::pause::{PausedThread, PausedThreadState};
pub use group::interrupt::{InterruptChange, InterruptGroupState};
pub use group::memory::address_space::{AddressSpace, AddressSpaceChange, AddressSpaceState};
pub use group::memory::page_table::{
    page_table_storage, PAGE_TABLE_STORAGE_BASE, PAGE_TABLE_STORAGE_BYTES,
};
pub use group::memory::region::{real_mode_size, MemoryRegion, MemoryState};
pub use group::memory::storage::{
    allocated_storage, ALLOCATED_STORAGE_BASE, ALLOCATED_STORAGE_BYTES,
};
pub use group::memory::{MemoryChange, MemoryGroupState};
pub use group::network::{NetworkChange, NetworkState};
pub use group::partition::repository::RepositoryRead;
pub use group::partition::{PartitionChange, PartitionState};
pub use group::spe::{
    shadow_storage, InterruptMask, InterruptMaskState, LogicalSpe, LogicalSpeState, RunTimeout,
    RunTimeoutState, SpeChange, SpeState, TransitionNotifier, TransitionNotifierState,
    SHADOW_STORAGE_BASE, SHADOW_STORAGE_PAGES,
};
pub use group::state::{GroupStates, StateChange};
pub use group::storage::{NotifyRequest, StorageChange, StorageState, WaitingRead};
pub use group::uart::{
    PendingRequest, ReceivedBytes, UartChange, UartState, VirtualUartIrq, VirtualUartPortParams,
    VirtualUartShadowWrite, UART_SHADOW_DOUBLEWORDS,
};
pub use handlers::{answer, answered_numbers, answers};
pub use interprocessor::{RunningThread, RunningThreads};
pub use mediated::MediatedTake;
pub use request::HypercallRequest;
pub use return_path::{
    Compare, ExternalDelivery, ExternalRaise, ReturnPair, ReturnRegisters, SavedMsrs,
};
pub use state::{HypercallWitness, Hypervisor};
