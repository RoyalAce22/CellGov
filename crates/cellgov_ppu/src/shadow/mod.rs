//! Predecoded instruction shadow for PT_LOAD text ranges.
//!
//! Fetch is a bounds check plus array index; slots are produced once
//! at construction, with a quickening and super-pairing pass applied
//! before first use.
//!
//! [ErtlGregg2003 p:4 s:2] flat sequential VM-code layout.
//! [Bala2000 p:2 s:2] code cache indexed by source-binary address.
//!
//! Self-modifying code (CRT0 relocations, HLE trampoline planting)
//! goes through [`PredecodedShadow::invalidate_range`] followed by
//! [`PredecodedShadow::refresh`]; the stale bit forces the caller
//! onto the raw fetch + decode path until the slot is repopulated
//! from committed memory.
//!
//! [Bala2000 p:3 s:4.1] flushable code cache.
//!
//! The two passes that run during `build` live in their own
//! submodules so each can be tested in isolation:
//!
//! - `quicken`: rewrite a single decoded instruction into a
//!   specialized variant (e.g. `addi r3, 0, imm` -> `Li`).
//! - `superpair`: fuse two adjacent instructions into a single
//!   super-instruction variant (e.g. `lwz` + `cmpwi` -> `LwzCmpwi`),
//!   replacing the second slot with `Consumed`.

mod model;
mod quicken;
mod superpair;

#[cfg(test)]
mod test_support;

pub use model::PredecodedShadow;
