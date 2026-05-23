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
            runner: format!("rpcs3-{:?}", config.decoder).to_lowercase(),
            steps: None,
        },
        // Region-extraction adapter does not surface raw TTY bytes
        // beyond the magic-tagged payload it parses. Step-2 ps3autotests
        // path will read TTY directly via its own helper.
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
            runner: format!("rpcs3-{:?}", decoder).to_lowercase(),
            steps: None,
        },
        tty_log: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_format_in_metadata() {
        let name = format!("rpcs3-{:?}", Rpcs3Decoder::Interpreter).to_lowercase();
        assert_eq!(name, "rpcs3-interpreter");
        let name = format!("rpcs3-{:?}", Rpcs3Decoder::Llvm).to_lowercase();
        assert_eq!(name, "rpcs3-llvm");
    }
}
