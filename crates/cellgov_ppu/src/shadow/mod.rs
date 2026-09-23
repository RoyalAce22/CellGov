//! Predecoded instruction shadow for PT_LOAD text ranges.
//!
//! Fetch uses a bounds check and an array index. Construction decodes
//! and quickens slots; [`PredecodedShadow::build`] also fuses eligible
//! pairs. [`PredecodedShadow::build_quickened`] leaves pairs separate.
//!
//! [ErtlGregg2003 p:4 s:2] flat sequential VM-code layout.
//! [Bala2000 p:2 s:2] code cache indexed by source-binary address.
//!
//! Self-modifying code (CRT0 relocations, HLE trampoline planting)
//! goes through [`PredecodedShadow::invalidate_range`] followed by
//! [`PredecodedShadow::refresh`]; a stale slot forces the caller onto
//! the raw fetch + decode path until it is repopulated from committed
//! memory.
//!
//! [Bala2000 p:7 s:6 Fragment Cache Management] flushable code cache.

mod model;
mod quicken;
mod superpair;

#[cfg(test)]
#[path = "tests/test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "tests/semantics_support.rs"]
mod semantics_support;

pub use model::PredecodedShadow;
