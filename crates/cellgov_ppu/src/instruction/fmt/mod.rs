//! Assembly-text rendering for [`PpuInstruction`].
//!
//! [`AsmText`] is a pure projection of an already-decoded
//! instruction: it never re-decodes raw words and never carries its
//! own opcode tables. Branch targets render as resolved absolute
//! addresses, which is why the adapter carries the instruction's own
//! `addr`.
//!
//! Output is ASCII-only and allocation-free in the steady state.
//! Every `match` over [`PpuInstruction`] is exhaustive with no `_`
//! arm: a new variant fails compilation here until it gets an
//! explicit rendering.
//!
//! [`PpuInstruction`]: super::PpuInstruction
//!
//! Extended mnemonics follow the PPC v2.02 Book I assembler
//! appendix; the `simplify` table is consulted before canonical
//! rendering, and each rewrite has an exact structural gate. Branch
//! `at`-hint bits are dropped (no `+`/`-` suffix is rendered).
// [PPC-Book1 p:154 s:B.2.4] at-bit prediction suffixes; assemblers
// default the at bits to 0b00, and this renderer drops them.

mod mnemonic;
mod render;
mod simplify;
mod text;

pub use text::AsmText;

#[cfg(test)]
#[path = "tests/fmt_tests.rs"]
mod tests;
