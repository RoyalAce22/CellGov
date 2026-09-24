//! Read-only PowerPC disassembler over `cellgov_ppu`'s segment reader
//! and disassembly stream.
//!
//! Used to investigate guest behavior at specific addresses without
//! booting the title. Output format: `addr  raw  decoded` per
//! instruction. The instruction stream goes to stdout; structural
//! diagnostics ("past segment end", overlap warnings, data heuristic)
//! go to stderr so a downstream tool can pipe stdout cleanly.

mod args;
mod entry;
mod stream;

#[cfg(test)]
#[path = "tests/test_support.rs"]
mod test_support;

pub(crate) use entry::run;
