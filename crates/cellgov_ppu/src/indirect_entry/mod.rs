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
//! The image names an address in these ways:
//! - a doubleword in the file bytes of a segment, such as a function
//!   descriptor or a TOC entry;
//! - a 32-bit word in the file bytes of a segment, which `lwz` extends
//!   with zeros and `lwa` extends with its sign into a complete address.
//!   The bound names both addresses;
//! - a constant that the code builds in a GPR on a path of fall-throughs
//!   and direct branches;
//! - an entry of a relative jump table: a 32-bit offset from a named
//!   address, or from the word after a `bcctr`.
//!
//! An indirect branch ignores bits 62 and 63 of its target. So for each
//! named address, the bound holds the word at the address with those
//! bits at 0.
//!
//! An `rfid` with MSR\[SF\] at 0 in the new MSR fetches from the
//! low-order 32 bits of SRR0. So for an `rfid` that can leave MSR\[IR\]
//! at 1, the bound holds a word for each SRR0 constant that the scan
//! knows. It holds the word that the `rfid` fetches under each MSR\[SF\]
//! that SRR1 can give.
//!
//! An indirect branch in 32-bit mode, with MSR\[SF\] and MSR bit 1 at 0,
//! ignores the high-order 32 bits of its target. An `rfid` or `mtmsrd`
//! that can clear both bits starts 32-bit mode. A `bcctr` or `bclr` that
//! control can reach from such a start by fall-throughs and direct
//! branches can enter the word at the low-order 32 bits of each address
//! that it can reach in 64-bit mode. The bound keeps that word for each
//! named address, also when no segment holds the address itself.
//!
//! Real mode also ignores EA bits 0 and 1. So an indirect branch in real
//! mode can reach the word at the real address of a named address. For
//! each doubleword and each 32-bit word of the file bytes, and for each
//! constant that the code builds, the bound keeps that word. It keeps the
//! word also when no segment holds the address itself. A jump table entry
//! names only the word at the address itself.
//!
//! The scan cannot tell a code address from another value of the same
//! size. So each 32-bit word names an address, also a count or a flag
//! that only looks like the real address of a word of the code.
//!
//! A function reaches each word that control can get to from its first
//! word with r2 unchanged. On each word that a function reaches, r2 can
//! hold the TOC that the function's descriptor gives. On a word that
//! functions with two different TOCs reach, r2 can hold each TOC that a
//! descriptor gives. The bound does not hold an address that the code
//! computes in another way, such as from a value it loads at run time.
//!
//! The scan also keeps these addresses:
//! - the target of each `bclr` whose LR holds a constant that a path of
//!   fall-throughs and direct branches builds;
//! - the target of each `bcctr` from each constant that a path to it
//!   leaves in CTR;
//! - each `bcctr` that a path reaches with a CTR that the scan cannot
//!   prove:
//!   - a CTR after a call;
//!   - a CTR after a decrement of a constant that the scan records;
//!   - a CTR from a GPR that can hold a constant that the scan dropped;
//! - each `bclr` that a path reaches with an LR that can hold a constant
//!   the scan dropped.
//!
//! [`CtrFlow`] gives the paths that bring CTR to a `bcctr`. [`JoinFlow`]
//! gives the constants that the paths to a branch target bring in the
//! GPRs and in LR. These limits apply to it:
//! - it does not follow a call into its routine;
//! - it keeps at most 16 constants of one register at a branch target.
//!   When the paths bring more, it drops them and marks the register as
//!   one that can hold a constant the scan dropped. The image does not
//!   name a dropped constant. The mark moves only with a copy of the
//!   register. So the scan does not record a dropped constant that a
//!   store and a load move, or that a call takes into its routine.
//!
//! A second scan finds where an `rfid`, `mtmsr` or `mtmsrd` can put
//! control into real mode. It forgets each constant at a word that
//! control can enter other than by a fall-through, so a constant that it
//! keeps is the value on every path to the word.
//!
//! A hypercall site resolves only on the paths that this bound lets the
//! scan model. So the bound can hold too many words. It must not hold
//! too few:
//! - a path of the hypercall walk ends at a word of the bound, and the
//!   site stays unresolved. So an extra word can cost a resolution,
//!   which is correct behaviour;
//! - a missing word is a defect, because a site can then resolve on a
//!   path that the scan does not model.
//!
//! For the same reason, a flow that drops a constant of a GPR, of LR or
//! of CTR must mark the register as lost. A store and a load, or a call,
//! can move a dropped constant without its mark, as the limits above
//! state. Each of those cases is a defect. Data that decodes as code can
//! add words to the bound. Data that decodes as code, or an extra word,
//! can also bring more constants of one register to a branch target than
//! [`JoinFlow`] keeps. Then [`JoinFlow`] can drop a constant that it
//! keeps without them. On each word that a function reaches, the bound
//! takes r2 to hold a TOC that a descriptor gives, or a constant that the
//! paths to the word leave in r2.
//!
//! [`CtrFlow`]: ctr_flow::CtrFlow
//! [`JoinFlow`]: join_flow::JoinFlow

mod bound;
mod constants;
mod ctr_flow;
mod join_flow;
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

#[cfg(test)]
#[path = "../tests/indirect_entry_rfid_low_word_tests.rs"]
mod rfid_low_word_tests;
