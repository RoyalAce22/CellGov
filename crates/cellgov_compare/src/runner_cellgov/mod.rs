//! CellGov runner adapter: produces `Observation` values from scenarios
//! ([`observe`]) or from long-running boots ([`observe_from_boot`]).
//! The two paths share a region extractor (see `region.rs`) but stay
//! separate because the testkit runner has no notion of process-exit,
//! hard faults, or HLE-driven termination that a boot reports.

mod boot;
mod region;
mod scenario;

pub use boot::{observe_from_boot, BootOutcome, BootOutcomeParseError};
pub use region::RegionDescriptor;
pub use scenario::{observe, observe_with_determinism_check, DeterminismError};
