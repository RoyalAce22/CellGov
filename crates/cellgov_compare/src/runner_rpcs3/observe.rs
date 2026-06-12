//! Free-function entry points: invoke RPCS3 and pack the extracted
//! regions into an [`Observation`], plus a non-invoking variant that
//! reads a saved TTY log.

use std::path::Path;

use crate::observation::{Observation, ObservationMetadata, ObservedOutcome};
use crate::runner_rpcs3::config::{
    ExtractionMethod, Rpcs3Config, Rpcs3Decoder, Rpcs3TestConfig, TtyRegion,
};
use crate::runner_rpcs3::dump::parse_dump;
use crate::runner_rpcs3::error::Rpcs3Error;
use crate::runner_rpcs3::invoke::invoke;
use crate::runner_rpcs3::tty::parse_tty_log;

/// Invoke RPCS3 headless, then extract regions via the configured method.
pub fn observe(config: &Rpcs3Config, test: &Rpcs3TestConfig) -> Result<Observation, Rpcs3Error> {
    let outcome = invoke(config, test)?;
    let memory_regions = match &test.extraction {
        ExtractionMethod::DumpFile { path, regions } => parse_dump(path, regions)?,
        ExtractionMethod::TtyLog { path, regions } => parse_tty_log(path, regions)?,
    };

    Ok(Observation {
        outcome,
        memory_regions,
        events: vec![],
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: config.decoder.as_runner_str().to_string(),
            steps: None,
        },
        tty_log: Vec::new(),
    })
}

/// Build an observation from a saved TTY log without invoking RPCS3;
/// the outcome is forced to `Completed`.
pub fn observe_from_tty(
    tty_path: &Path,
    regions: &[TtyRegion],
    decoder: Rpcs3Decoder,
) -> Result<Observation, Rpcs3Error> {
    let memory_regions = parse_tty_log(tty_path, regions)?;
    Ok(Observation {
        outcome: ObservedOutcome::Completed,
        memory_regions,
        events: vec![],
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: decoder.as_runner_str().to_string(),
            steps: None,
        },
        tty_log: Vec::new(),
    })
}

#[cfg(test)]
#[path = "tests/observe_tests.rs"]
mod tests;
