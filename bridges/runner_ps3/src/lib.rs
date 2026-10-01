//! The PS3 runner: deploys a packaged microtest to a retail console
//! over webMAN, starts it, fetches the CGOV frame it leaves behind,
//! and converts the frame into the observation the comparison harness
//! reads.
//!
//! The console speaks HTTP/1.0 and anonymous FTP, so the transport is
//! two small clients over [`std::net::TcpStream`]; every exchange goes
//! through a [`transport::Wire`] so the unit tests drive the clients
//! from byte buffers. This crate is host tooling: it never runs guest
//! code and never touches the runtime.
//!
//! The verbs live in the binary; this library holds what they share.

pub mod capture;
pub mod cli;
pub mod console;
pub mod deploy;
pub mod error;
pub mod lease;
pub mod provenance;
pub mod run;
pub mod transcript;
pub mod transport;

pub use error::{ExitCode, RunnerPs3Error};

#[cfg(test)]
#[path = "tests/memory_console.rs"]
mod memory_console;
