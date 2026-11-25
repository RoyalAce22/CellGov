//! The hypervisor's own records: the null-backend witnesses, the guest
//! TTY sink, the repository reads and the refusal that stopped the run.

use std::collections::BTreeMap;

use cellgov_ps3_abi::lv1::call::READ_REPOSITORY_NODE;
use cellgov_time::GuestTicks;

use super::answer::{HypercallDisposition, HypercallRefusal};
use super::handlers::answer;
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

/// CellGov's hypervisor, the firmware version it answers for, and its
/// records.
///
/// No answer depends on the records, so they hold no guest state. The
/// firmware version selects the repository values a read answers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hypervisor {
    firmware: Option<String>,
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

    /// The disposition of `request`, after the records note it:
    /// - a console write appends its bytes to the TTY sink;
    /// - a repository read appends its record, answered or refused;
    /// - a number with no handler counts toward its witness;
    /// - the first refusal is kept.
    pub fn dispatch(&mut self, request: &HypercallRequest) -> HypercallDisposition {
        let disposition = answer(request, self.firmware.as_deref());
        if request.number == READ_REPOSITORY_NODE {
            self.repository_reads
                .push(read_record(request, &disposition));
        }
        match &disposition {
            HypercallDisposition::Answered(answer) => {
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
