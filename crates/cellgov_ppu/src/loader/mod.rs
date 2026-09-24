//! PPU ELF64 loader: copies PT_LOAD segments into guest memory and
//! resolves the entry-point OPD into `(pc, toc)`.

mod load;
mod opd_tables;
mod phdr;
mod process_param;
mod symbols;
mod tls;

pub(crate) use cellgov_mem::be::{read_u16, read_u32, read_u64};

pub use load::*;
pub use opd_tables::*;
pub use phdr::*;
pub use process_param::*;
pub use symbols::*;
pub use tls::*;

#[cfg(test)]
#[path = "tests/loader_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/loader_pt_load_tests.rs"]
mod pt_load_tests;
