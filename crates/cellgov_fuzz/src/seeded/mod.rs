//! Seeded target defects that prove each check detects what it claims to detect.
//!
//! Every hook in this module is an identity outside test builds. A test seeds
//! one defect for its thread, runs an engine through the public entry points,
//! and asserts the finding kind that defect must produce; the same campaign
//! without the defect must stay clean. Simple bugs seeded into the
//! implementation and found within minutes validate the fuzzing
//! configuration itself. [Watt2023 p:110:20 s:5.2] A defect that reaches
//! every run of a self-differential check the same way is invisible to it,
//! so an independent reference remains a separate tier. [Watt2023 p:110:2 s:1]
//! Seeded disagreement, crash, and corrupted state prove each comparison tier
//! can detect them. [McKeeman1998 p:101 s:Differential Testing]
//!
//! Each hook sits at the boundary its defect corrupts, so removing the check
//! that boundary feeds turns exactly one named test red:
//!
//! | Boundary                      | Defects                              | Check that must fire        |
//! | ----------------------------- | ------------------------------------ | --------------------------- |
//! | decoder and executor calls    | `DecoderPanic`, `ExecutorPanic`      | target-panic capture        |
//! | encoder call                  | `EncoderMismatch`                    | census round-trip class     |
//! | first-run outcome class       | `IllegalOutcome`                     | legal-outcome contract      |
//! | first-run effect classes      | `IllegalEffect`                      | legal-effect contract       |
//! | SPU registers after execution | `IllegalFootprint`, `CommonMode`     | allowed footprint, or none  |
//! | SPU sequence program counter  | `InvalidProgramCounter`              | program-counter contract    |
//! | replay run only               | `Nondeterministic`                   | deterministic replay        |
//! | metamorphic partner only      | `MetamorphicMismatch`                | declared relation           |
//! | every PPU observation         | `CommonMode`                         | none: the reference tier    |

#[cfg(test)]
mod defect;
#[cfg(not(test))]
mod disabled;
#[cfg(test)]
mod enabled;

#[cfg(test)]
pub(crate) use defect::SeededDefect;
#[cfg(not(test))]
pub(crate) use disabled::*;
#[cfg(test)]
pub(crate) use enabled::*;

#[cfg(test)]
#[path = "../tests/seeded_tests.rs"]
mod tests;
