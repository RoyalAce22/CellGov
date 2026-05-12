//! RPCS3 runner adapter: invokes the patched RPCS3 binary headless,
//! then extracts the microtest result from either a binary memory dump
//! or the RPCS3 TTY log, and packs it into an `Observation`.

mod config;
mod dump;
mod error;
mod invoke;
mod tty;

pub use config::{
    DumpRegion, ExtractionMethod, Rpcs3Config, Rpcs3Decoder, Rpcs3TestConfig, TtyRegion,
};
pub use dump::parse_dump;
pub use error::Rpcs3Error;
pub use tty::{parse_tty_log, TTY_MAGIC};

use std::path::Path;

use crate::observation::{Observation, ObservationMetadata, ObservedOutcome};

use config::ExtractionMethod as _ExtractionMethod;
use invoke::invoke;

/// Invoke RPCS3 headless, then extract regions via the configured method.
pub fn observe(config: &Rpcs3Config, test: &Rpcs3TestConfig) -> Result<Observation, Rpcs3Error> {
    let outcome = invoke(config, test)?;
    let memory_regions = match &test.extraction {
        _ExtractionMethod::DumpFile { path, regions } => parse_dump(path, regions)?,
        _ExtractionMethod::TtyLog { path, regions } => parse_tty_log(path, regions)?,
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
