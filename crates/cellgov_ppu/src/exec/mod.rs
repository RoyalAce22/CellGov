//! PPU instruction dispatch: decodes [`crate::instruction::PpuInstruction`]
//! variants to per-unit submodules, mutating [`crate::state::PpuState`]
//! and staging memory [`cellgov_effects::Effect`]s. Syscalls escape
//! via [`ExecuteVerdict::Syscall`].
//!
//! Memory-touching vector ops (`lvx`, `lvlx`, `lvrx`, `stvx`) route
//! through `mem` rather than `vec` so every load / store shares
//! one store-buffer-forward / region-view path.

mod alu;
mod branch;
mod cr;
mod dispatch;
mod fault;
mod mem;
mod memory_helpers;
mod super_insn;
mod vec;
mod verdict;

#[cfg(test)]
#[path = "tests/test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "tests/exec_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/fault_leaves_state_tests.rs"]
mod fault_leaves_state_tests;

#[cfg(test)]
#[path = "tests/cache_block_tests.rs"]
mod cache_block_tests;

#[cfg(test)]
#[path = "tests/cache_block_console_tests.rs"]
mod cache_block_console_tests;

pub use dispatch::execute;
pub use fault::PpuFault;
pub use verdict::ExecuteVerdict;
