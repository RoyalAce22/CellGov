//! The hypervisor's state: the partition's memory regions, virtual
//! address spaces and interrupt state bitmaps, and its own records of
//! the witnesses of numbers with no handler, the guest TTY sink, the
//! repository reads and the refusal that stopped the run.

use std::collections::BTreeMap;

use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::READ_REPOSITORY_NODE;
use cellgov_time::GuestTicks;

use super::address_space::{AddressSpaceChange, AddressSpaceState};
use super::answer::{HypercallDisposition, HypercallRefusal};
use super::handlers::{answer, answers};
use super::interrupt::IrqStateBitmapState;
use super::memory::MemoryState;
use super::repository::{read_record, RepositoryRead};
use super::request::HypercallRequest;
use super::vector;

/// How often the hypervisor saw one hypercall number that no handler
/// answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HypercallWitness {
    /// Calls of the number.
    pub hits: u64,
    /// The guest time of the first call.
    pub first_tick: GuestTicks,
}

/// CellGov's hypervisor, the firmware version it answers for, the
/// partition's memory regions, virtual address spaces and interrupt
/// state bitmaps, and its records.
///
/// The firmware version selects the values that the repository reads,
/// the allocations and the address space calls answer. The answers
/// depend on the memory regions, the address spaces and the bitmaps, so
/// they are guest state, and [`Self::sync_term`] hashes them. No answer
/// depends on the records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hypervisor {
    firmware: Option<String>,
    memory: MemoryState,
    address_spaces: AddressSpaceState,
    irq_state_bitmaps: IrqStateBitmapState,
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

    /// Makes the hypervisor answer repository reads for the firmware
    /// version `version`, such as `"4.93"`.
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
    /// - a created memory region joins the partition's regions;
    /// - a change to the address spaces applies, and a page table
    ///   mapping moves the region counter past the value it took;
    /// - a registered interrupt state bitmap joins the bitmaps;
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
        let disposition = answer(
            request,
            self.firmware.as_deref(),
            &self.memory,
            &self.address_spaces,
            &self.irq_state_bitmaps,
            storage,
        );
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
                if let Some(region) = answer.region {
                    self.memory.create(region);
                }
                if let Some(change) = answer.address_space {
                    if let AddressSpaceChange::Mapped { counter, .. } = change {
                        self.memory.take_counter(counter);
                    }
                    self.address_spaces.apply(change);
                }
                if let Some(registration) = answer.irq_state_bitmap {
                    self.irq_state_bitmaps.register(registration);
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

    /// The partition's memory regions and its region counter.
    pub fn memory(&self) -> &MemoryState {
        &self.memory
    }

    /// The partition's virtual address spaces and the selected one.
    pub fn address_spaces(&self) -> &AddressSpaceState {
        &self.address_spaces
    }

    /// The interrupt state bitmap that each logical PPE thread
    /// registered.
    pub fn irq_state_bitmaps(&self) -> &IrqStateBitmapState {
        &self.irq_state_bitmaps
    }

    /// The sync-state terms of the hypervisor's guest state: its memory
    /// regions, its address spaces and its interrupt state bitmaps. The
    /// records add no term.
    pub fn sync_term(&self) -> u128 {
        self.memory
            .sync_term()
            .wrapping_add(self.address_spaces.sync_term())
            .wrapping_add(self.irq_state_bitmaps.sync_term())
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
