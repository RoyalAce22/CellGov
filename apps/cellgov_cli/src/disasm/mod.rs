//! Read-only PowerPC disassembler that delegates decoding to
//! `cellgov_ppu::decode::decode`.
//!
//! Used to investigate guest behavior at specific addresses without
//! booting the title. Output format: `addr  raw  decoded` per
//! instruction. The instruction stream goes to stdout; structural
//! diagnostics ("past segment end", overlap warnings, data heuristic)
//! go to stderr so a downstream tool can pipe stdout cleanly.

mod args;
mod elf;
mod entry;
mod stream;

#[cfg(test)]
#[path = "tests/test_support.rs"]
mod test_support;

pub(crate) use entry::run;
