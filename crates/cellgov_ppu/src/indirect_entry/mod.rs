//! Bounds the words of an ELF that control can enter in a way other than
//! a direct branch or a fall-through.
//!
//! A word is in the bound when one of these can enter it:
//! - the start of execution, at the ELF entry;
//! - an interrupt, at a word whose real address is in the interrupt
//!   vectors;
//! - an indirect branch (`bcctr`, `bclr`) or an `rfid`, at an address
//!   that the image names.
//!
//! The image names an address in three ways:
//! - a doubleword in the file bytes of a segment, such as a function
//!   descriptor or a TOC entry;
//! - a constant that the code builds in a GPR on a straight-line path;
//! - an entry of a relative jump table: a 32-bit offset from a named
//!   address, or from the word after a `bcctr`.
//!
//! An indirect branch ignores bits 62 and 63 of its target. So for each
//! named address, the bound holds the word at the address with those
//! bits at 0.
//!
//! Real mode also ignores EA bits 0 and 1. So for each doubleword of the
//! file bytes and each constant that the code builds, the bound also
//! keeps each word at its real address, which an indirect branch in real
//! mode can reach. The bound keeps such a word also when no segment
//! holds the address itself. A jump table entry names only the word at
//! the address itself.
//!
//! A function reaches each word that control can get to from its first
//! word with r2 unchanged. On each word that a function reaches, r2 can
//! hold the TOC that the function's descriptor gives. On a word that
//! functions with two different TOCs reach, r2 can hold each TOC that a
//! descriptor gives. The bound does not hold an address that the code
//! computes in another way, such as from a value it loads at run time.
//!
//! The scan also keeps these addresses:
//! - the target of each `bclr` whose LR holds a constant that a
//!   straight-line path builds;
//! - the target of each `bcctr` from each constant that a path to it
//!   leaves in CTR;
//! - each `bcctr` that a path reaches with a CTR that the scan cannot
//!   prove: a CTR after a call, or after a decrement of a constant that
//!   the scan records.
//!
//! [`CtrFlow`] gives the paths that bring CTR to a `bcctr`.
//!
//! A second scan finds where an `rfid`, `mtmsr` or `mtmsrd` can put
//! control into real mode. It forgets each constant at a word that
//! control can enter other than by a fall-through, so a constant that it
//! keeps is the value on every path to the word.
//!
//! [`CtrFlow`]: ctr_flow::CtrFlow

mod bound;
mod constants;
mod ctr_flow;
mod real_mode;
mod toc;

pub(crate) use bound::{indirect_entries, FileSegment, IndirectBound, JUMP_TABLE_LIMIT};
pub(crate) use real_mode::real_mode_entries;

// The test modules take these names from this module's scope.
#[cfg(test)]
use std::collections::BTreeSet;

#[cfg(test)]
use cellgov_ps3_abi::format::elf::ppc64_function_descriptor as descriptor;
#[cfg(test)]
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_SF, SPR_SRR0, SPR_SRR1};

#[cfg(test)]
use crate::caller_census::CallerScanError;
#[cfg(test)]
use crate::hypercall_cfg::{Code, CodeSegment};
#[cfg(test)]
use crate::instruction::PpuInstruction;

#[cfg(test)]
#[path = "../tests/indirect_entry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/indirect_entry_partial_constant_tests.rs"]
mod partial_constant_tests;

#[cfg(test)]
#[path = "../tests/indirect_entry_toc_tests.rs"]
mod toc_tests;

#[cfg(test)]
#[path = "../tests/indirect_entry_path_end_tests.rs"]
mod path_end_tests;

#[cfg(test)]
#[path = "../tests/indirect_entry_function_toc_tests.rs"]
mod function_toc_tests;

#[cfg(test)]
#[path = "../tests/indirect_entry_toc_candidate_tests.rs"]
mod toc_candidate_tests;
