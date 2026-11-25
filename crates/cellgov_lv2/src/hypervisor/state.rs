//! The hypervisor's own records: the null-backend witnesses, the guest
//! TTY sink and the refusal that stopped the run.

use std::collections::BTreeMap;

use cellgov_time::GuestTicks;

use super::answer::{HypercallDisposition, HypercallRefusal};
use super::handlers::answer;
use super::request::HypercallRequest;

/// How often the null backend saw one hypercall number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HypercallWitness {
    /// Calls of the number.
    pub hits: u64,
    /// The guest time of the first call.
    pub first_tick: GuestTicks,
}

/// CellGov's hypervisor and its records.
///
/// No answer depends on these records, so they hold no guest state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hypervisor {
    witnesses: BTreeMap<u64, HypercallWitness>,
    console: Vec<u8>,
    refusal: Option<HypercallRefusal>,
}

impl Hypervisor {
    /// A hypervisor with empty records.
    pub fn new() -> Self {
        Self::default()
    }

    /// The disposition of `request`, after the records note it:
    /// - a console write appends its bytes to the TTY sink;
    /// - a number with no handler counts toward its witness;
    /// - the first refusal is kept.
    pub fn dispatch(&mut self, request: &HypercallRequest) -> HypercallDisposition {
        let disposition = answer(request);
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

    /// The first refusal, which stops the run.
    pub fn refusal(&self) -> Option<HypercallRefusal> {
        self.refusal
    }
}

#[cfg(test)]
#[path = "tests/state_tests.rs"]
mod tests;
