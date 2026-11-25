//! One batch: the fetch-decode-execute loop and its yields.

mod clock;
mod decode;
mod exit;
mod miss;
mod regions;
mod retire;
mod run;

pub(super) use decode::{fetch_window, CodeBytes};
pub(super) use exit::FaultExit;
pub(super) use miss::MissStep;
pub(super) use run::{BatchEntry, NoTap};

#[cfg(test)]
pub(super) use clock::SIBLING_LEAD_CYCLES;

#[cfg(test)]
#[path = "tests/translation_hit_miss_tests.rs"]
mod translation_hit_miss_tests;

#[cfg(test)]
#[path = "tests/faulted_reserve_count_tests.rs"]
mod faulted_reserve_count_tests;

#[cfg(test)]
#[path = "tests/unbacked_hit_trim_tests.rs"]
mod unbacked_hit_trim_tests;
