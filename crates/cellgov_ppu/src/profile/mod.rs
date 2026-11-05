//! The operand-class profile that ranks predecoded-shadow candidates.
//!
//! A profiled unit counts the raw words it retires, so a pair the shadow
//! fuses still counts as its two instructions. The profile keys each count
//! on the decoded variant and the operand classes that a quickening or a
//! fused pair can specialize on. The profile is host-side: it reads no
//! guest state the run does not already hold, and it is outside the state
//! hash.

mod class;
mod counts;
mod load;
mod report;

pub use class::{BaseClass, InsnClass, PairClass};
pub use counts::OperandProfile;
pub use load::{LoadOutcome, LoadProfile, LoadSummary};
pub use report::{
    coverage, report_units, shipped_coverage, shipped_forms, static_counts, Coverage,
    ProfileReport, WordSource,
};

#[cfg(test)]
#[path = "tests/class_tests.rs"]
mod class_tests;

#[cfg(test)]
#[path = "tests/report_tests.rs"]
mod report_tests;

#[cfg(test)]
#[path = "tests/shipped_forms_tests.rs"]
mod shipped_forms_tests;

#[cfg(test)]
#[path = "tests/merge_tests.rs"]
mod merge_tests;

#[cfg(test)]
#[path = "tests/load_tests.rs"]
mod load_tests;

#[cfg(test)]
#[path = "tests/region_words_tests.rs"]
mod region_words_tests;
