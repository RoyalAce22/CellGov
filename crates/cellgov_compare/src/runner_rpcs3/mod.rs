//! RPCS3 runner adapter: invokes the patched RPCS3 binary headless,
//! then extracts the microtest result from either a binary memory dump
//! or the RPCS3 TTY log, and packs it into an `Observation`.

mod config;
mod dump;
mod error;
mod invoke;
mod observe;
mod tty;

pub use config::{
    DumpRegion, ExtractionMethod, Rpcs3Config, Rpcs3Decoder, Rpcs3TestConfig, TtyRegion,
};
pub use dump::parse_dump;
pub use error::Rpcs3Error;
pub use observe::{observe, observe_from_tty};
pub use tty::{parse_tty_log, TTY_MAGIC};
