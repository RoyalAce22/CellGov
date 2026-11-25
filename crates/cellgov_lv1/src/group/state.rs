//! The state of each hypercall group, and the changes an answer carries.

use super::device_bus::{DeviceBusChange, DeviceBusState};
use super::gpu::{GpuChange, GpuState};
use super::interrupt::{InterruptChange, InterruptGroupState};
use super::memory::{MemoryChange, MemoryGroupState};
use super::network::{NetworkChange, NetworkState};
use super::partition::{PartitionChange, PartitionState};
use super::spe::{SpeChange, SpeState};
use super::storage::{StorageChange, StorageState};
use super::uart::{UartChange, UartState};

/// The guest state of each hypercall group.
///
/// Each group defines its own state type in its own module. A group with
/// no state holds an empty one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupStates {
    memory: MemoryGroupState,
    interrupt: InterruptGroupState,
    partition: PartitionState,
    device_bus: DeviceBusState,
    spe: SpeState,
    storage: StorageState,
    gpu: GpuState,
    network: NetworkState,
    uart: UartState,
}

impl GroupStates {
    /// The memory and page table group's state.
    pub fn memory(&self) -> &MemoryGroupState {
        &self.memory
    }

    /// The interrupt delivery group's state.
    pub fn interrupt(&self) -> &InterruptGroupState {
        &self.interrupt
    }

    /// The partition, console and time group's state.
    pub fn partition(&self) -> &PartitionState {
        &self.partition
    }

    /// The device bus group's state.
    pub fn device_bus(&self) -> &DeviceBusState {
        &self.device_bus
    }

    /// The logical SPE group's state.
    pub fn spe(&self) -> &SpeState {
        &self.spe
    }

    /// The storage group's state.
    pub fn storage(&self) -> &StorageState {
        &self.storage
    }

    /// The GPU group's state.
    pub fn gpu(&self) -> &GpuState {
        &self.gpu
    }

    /// The network device group's state.
    pub fn network(&self) -> &NetworkState {
        &self.network
    }

    /// The virtual UART group's state.
    pub fn uart(&self) -> &UartState {
        &self.uart
    }

    /// Applies `change` to the state of the group that owns it.
    pub(crate) fn apply(&mut self, change: StateChange) {
        match change {
            StateChange::Memory(change) => self.memory.apply(*change),
            StateChange::Interrupt(change) => self.interrupt.apply(*change),
            StateChange::Partition(change) => self.partition.apply(*change),
            StateChange::DeviceBus(change) => self.device_bus.apply(*change),
            StateChange::Spe(change) => self.spe.apply(*change),
            StateChange::Storage(change) => self.storage.apply(*change),
            StateChange::Gpu(change) => self.gpu.apply(*change),
            StateChange::Network(change) => self.network.apply(*change),
            StateChange::Uart(change) => self.uart.apply(*change),
        }
    }

    /// The sum of each group's sync-state terms.
    pub fn sync_term(&self) -> u128 {
        [
            self.memory.sync_term(),
            self.interrupt.sync_term(),
            self.partition.sync_term(),
            self.device_bus.sync_term(),
            self.spe.sync_term(),
            self.storage.sync_term(),
            self.gpu.sync_term(),
            self.network.sync_term(),
            self.uart.sync_term(),
        ]
        .into_iter()
        .fold(0, u128::wrapping_add)
    }
}

/// A change to one group's state that an answer carries, which the
/// hypervisor applies.
///
/// Each group defines its own change type in its own module. The box
/// keeps the size of this enum and of [`crate::HypercallDisposition`]
/// fixed when a group's type grows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateChange {
    /// A change to the memory and page table group's state.
    Memory(Box<MemoryChange>),
    /// A change to the interrupt delivery group's state.
    Interrupt(Box<InterruptChange>),
    /// A change to the partition, console and time group's state.
    Partition(Box<PartitionChange>),
    /// A change to the device bus group's state.
    DeviceBus(Box<DeviceBusChange>),
    /// A change to the logical SPE group's state.
    Spe(Box<SpeChange>),
    /// A change to the storage group's state.
    Storage(Box<StorageChange>),
    /// A change to the GPU group's state.
    Gpu(Box<GpuChange>),
    /// A change to the network device group's state.
    Network(Box<NetworkChange>),
    /// A change to the virtual UART group's state.
    Uart(Box<UartChange>),
}
