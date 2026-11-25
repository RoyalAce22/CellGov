//! The hypervisor's state: the partition's memory regions, and its own
//! records of the null-backend witnesses, the guest TTY sink, the
//! repository reads and the refusal that stopped the run.

use std::collections::BTreeMap;

use cellgov_ps3_abi::lv1::call::READ_REPOSITORY_NODE;
use cellgov_time::GuestTicks;

use super::answer::{HypercallDisposition, HypercallRefusal};
use super::handlers::answer;
use super::memory::MemoryState;
use super::repository::{read_record, RepositoryRead};
use super::request::HypercallRequest;

/// How often the null backend saw one hypercall number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HypercallWitness {
    /// Calls of the number.
    pub hits: u64,
    /// The guest time of the first call.
    pub first_tick: GuestTicks,
}

/// CellGov's hypervisor, the firmware version it answers for, the
/// partition's memory regions, and its records.
///
/// The firmware version selects the values that the repository reads
/// and the allocations answer. The answers depend on the memory regions,
/// so the regions are guest state, and [`Self::sync_term`] hashes them.
/// No answer depends on the records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hypervisor {
    firmware: Option<String>,
    memory: MemoryState,
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

    /// The disposition of `request`, after the state and the records note
    /// it:
    /// - a created memory region joins the partition's regions;
    /// - a console write appends its bytes to the TTY sink;
    /// - a repository read appends its record, answered or refused;
    /// - a number with no handler counts toward its witness;
    /// - the first refusal is kept.
    pub fn dispatch(&mut self, request: &HypercallRequest) -> HypercallDisposition {
        let disposition = answer(request, self.firmware.as_deref(), &self.memory);
        if request.number == READ_REPOSITORY_NODE {
            self.repository_reads
                .push(read_record(request, &disposition));
        }
        match &disposition {
            HypercallDisposition::Answered(answer) => {
                if let Some(region) = answer.region {
                    self.memory.create(region);
                }
                if let Some(write) = &answer.console {
                    self.console.extend_from_slice(write.bytes());
                }
            }
            HypercallDisposition::Refused(refusal) => {
                if let HypercallRefusal::Unmodelled { number, .. } = refusal {
                    self.witnesses
                        .entry(*number)
                        .and_modify(|w| w.hits = w.hits.saturating_add(1))
                        .or_insert(HypercallWitness {
                            hits: 1,
                            first_tick: request.tick,
                        });
                }
                self.refusal.get_or_insert(*refusal);
            }
        }
        disposition
    }

    /// The partition's memory regions and its region counter.
    pub fn memory(&self) -> &MemoryState {
        &self.memory
    }

    /// The sync-state terms of the hypervisor's guest state: its memory
    /// regions. The records add no term.
    pub fn sync_term(&self) -> u128 {
        self.memory.sync_term()
    }

    /// Each number the null backend saw, ascending.
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
