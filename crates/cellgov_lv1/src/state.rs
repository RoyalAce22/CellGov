//! The hypervisor's state: the state of each hypercall group, and its
//! own records of the witnesses of numbers with no handler, the guest
//! TTY sink, the repository reads and the refusal that stopped the run.

use std::collections::BTreeMap;

use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::READ_REPOSITORY_NODE;
use cellgov_time::GuestTicks;

use super::answer::{HypercallDisposition, HypercallRefusal};
use super::handlers::{answer, answers};
use super::request::HypercallRequest;
use super::vector;
use crate::group::interrupt::bitmap::IrqStateBitmapState;
use crate::group::interrupt::event_port::EventReceivePortState;
use crate::group::memory::address_space::AddressSpaceState;
use crate::group::memory::region::MemoryState;
use crate::group::partition::repository::{read_record, RepositoryRead};
use crate::group::state::GroupStates;

/// How often the hypervisor saw one hypercall number that no handler
/// answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HypercallWitness {
    /// Calls of the number.
    pub hits: u64,
    /// The guest time of the first call.
    pub first_tick: GuestTicks,
}

/// CellGov's hypervisor, the firmware version it answers for, the state
/// of each hypercall group, and its records.
///
/// The firmware version selects the values that the repository reads,
/// the allocations and the address space calls answer. The answers
/// depend on the group states, so the group states are guest state, and
/// [`Self::sync_term`] hashes them. No answer depends on the records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hypervisor {
    firmware: Option<String>,
    groups: GroupStates,
    witnesses: BTreeMap<u64, HypercallWitness>,
    console: Vec<u8>,
    repository_reads: Vec<RepositoryRead>,
    refusal: Option<HypercallRefusal>,
}

impl Hypervisor {
    /// A hypervisor with empty records and no firmware version. It
    /// refuses every repository read.
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes the hypervisor answer for the firmware version `version`,
    /// such as `"4.93"`.
    pub fn set_firmware(&mut self, version: &str) {
        self.firmware = Some(version.to_string());
    }

    /// The firmware version the hypervisor answers for.
    pub fn firmware(&self) -> Option<&str> {
        self.firmware.as_deref()
    }

    /// The disposition of `request` from a caller with no real storage,
    /// as [`Self::dispatch_with`] gives it. Each page table call refuses.
    pub fn dispatch(&mut self, request: &HypercallRequest) -> HypercallDisposition {
        self.dispatch_with(request, &GuestMemory::new(0))
    }

    /// The disposition of `request` from a caller whose real address
    /// space is `storage`, after the state and the records note it:
    /// - each change the answer carries applies to its group's state, in
    ///   order;
    /// - a console write appends its bytes to the TTY sink;
    /// - a repository read that reaches its handler appends its record,
    ///   answered or refused;
    /// - a number with no handler counts toward its witness, whether
    ///   lv1's answer or a refusal disposes of it;
    /// - the first refusal is kept.
    pub fn dispatch_with(
        &mut self,
        request: &HypercallRequest,
        storage: &GuestMemory,
    ) -> HypercallDisposition {
        let disposition = answer(request, self.firmware.as_deref(), &self.groups, storage);
        // A read that lv1's vector answers or refuses never reaches the
        // node table, so it is no repository read.
        if request.number == READ_REPOSITORY_NODE
            && vector::vector_disposition(request, self.firmware.as_deref()).is_none()
        {
            self.repository_reads
                .push(read_record(request, &disposition));
        }
        if !answers(request.number) {
            self.witnesses
                .entry(request.number)
                .and_modify(|w| w.hits = w.hits.saturating_add(1))
                .or_insert(HypercallWitness {
                    hits: 1,
                    first_tick: request.tick,
                });
        }
        match &disposition {
            HypercallDisposition::Answered(answer) => {
                for change in &answer.changes {
                    self.groups.apply(change.clone());
                }
                if let Some(write) = &answer.console {
                    self.console.extend_from_slice(write.bytes());
                }
            }
            HypercallDisposition::Refused(refusal) => {
                self.refusal.get_or_insert(*refusal);
            }
        }
        disposition
    }

    /// The state of each hypercall group.
    pub fn groups(&self) -> &GroupStates {
        &self.groups
    }

    /// The partition's memory regions and its region counter.
    pub fn memory(&self) -> &MemoryState {
        self.groups.memory().regions()
    }

    /// The partition's virtual address spaces and the selected one.
    pub fn address_spaces(&self) -> &AddressSpaceState {
        self.groups.memory().spaces()
    }

    /// The interrupt state bitmap that each logical PPE thread
    /// registered.
    pub fn irq_state_bitmaps(&self) -> &IrqStateBitmapState {
        self.groups.interrupt().bitmaps()
    }

    /// The partition's event receive ports.
    pub fn event_receive_ports(&self) -> &EventReceivePortState {
        self.groups.interrupt().ports()
    }

    /// The sync-state terms of the hypervisor's guest state: the state of
    /// each hypercall group. The records add no term.
    pub fn sync_term(&self) -> u128 {
        self.groups.sync_term()
    }

    /// Each number with no handler that the hypervisor saw, ascending.
    pub fn witnesses(&self) -> &BTreeMap<u64, HypercallWitness> {
        &self.witnesses
    }

    /// Every byte the logical console took, in order.
    pub fn console_text(&self) -> &[u8] {
        &self.console
    }

    /// Every repository read, in the order the kernel made them.
    pub fn repository_reads(&self) -> &[RepositoryRead] {
        &self.repository_reads
    }

    /// The first refusal, which stops the run.
    pub fn refusal(&self) -> Option<HypercallRefusal> {
        self.refusal
    }
}

#[cfg(test)]
#[path = "tests/state_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/repository_log_tests.rs"]
mod repository_log_tests;
