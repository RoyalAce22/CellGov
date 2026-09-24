//! A PPU ELF's instruction stream as values: one item per word, from
//! an address in a loaded segment to where the segment's file-backed
//! bytes end.
//!
//! [`Disassembly`] decides where the stream stops and where functions
//! start, and decodes each word. It writes nothing: a disassembler
//! renders the items as text, and a static recompiler consumes them
//! directly.

use crate::decode::decode;
use crate::funcmap::{FunctionMap, FunctionSpan};
use crate::instruction::PpuInstruction;
use crate::loader::LoadSegment;

/// One item of the stream.
#[derive(Debug, Clone, PartialEq)]
pub enum DisasmItem<'a> {
    /// The next word starts a function of the map the stream was given.
    /// The word itself follows as the next item.
    FunctionStart(&'a FunctionSpan),
    /// A word that decodes.
    Instruction {
        /// The word's address.
        addr: u64,
        /// The word as stored, big-endian.
        raw: u32,
        /// What it decodes to.
        insn: PpuInstruction,
    },
    /// A word that does not decode.
    Undecodable {
        /// The word's address.
        addr: u64,
        /// The word as stored, big-endian.
        raw: u32,
    },
    /// The stream ends here, and yields nothing after.
    End(DisasmEnd),
}

/// Why the stream ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisasmEnd {
    /// The word at `addr` lies in the segment past its file-backed
    /// bytes: zero-fill (BSS), nothing to decode.
    ZeroFill {
        /// The first address with no file byte behind it.
        addr: u64,
    },
    /// The word at `addr` is outside the segment's bytes: past its end,
    /// below its start, or past the end of the file that holds them.
    SegmentEnd {
        /// The address of that word.
        addr: u64,
    },
    /// The address of word `words` does not fit u64.
    AddressOverflow {
        /// How many words the stream yielded before it.
        words: u64,
    },
}

/// The words of one segment from a start address, in address order.
///
/// Each word yields [`DisasmItem::Instruction`] or
/// [`DisasmItem::Undecodable`], preceded by [`DisasmItem::FunctionStart`]
/// when a function of `symbols` starts at it. The stream ends with one
/// [`DisasmItem::End`]: at the first word past the file-backed bytes,
/// past the segment, or past the top of the address space.
pub struct Disassembly<'a> {
    elf: &'a [u8],
    segment: LoadSegment,
    start: u64,
    words: u64,
    symbols: Option<&'a FunctionMap>,
    /// The word item a [`DisasmItem::FunctionStart`] was yielded ahead
    /// of.
    pending: Option<DisasmItem<'a>>,
    done: bool,
}

impl<'a> Disassembly<'a> {
    /// The words of `segment` from `start`, read from `elf`, the file
    /// `segment` was read from.
    ///
    /// `start` is expected to lie in the segment's file-backed bytes;
    /// [`crate::loader::address_source`] finds that segment. A start
    /// below the segment ends the stream at once.
    #[must_use]
    pub fn new(
        elf: &'a [u8],
        segment: LoadSegment,
        start: u64,
        symbols: Option<&'a FunctionMap>,
    ) -> Self {
        Self {
            elf,
            segment,
            start,
            words: 0,
            symbols,
            pending: None,
            done: false,
        }
    }

    fn end(&mut self, end: DisasmEnd) -> Option<DisasmItem<'a>> {
        self.done = true;
        Some(DisasmItem::End(end))
    }

    /// The word at `addr`, when the segment's file bytes hold all four
    /// of its bytes; otherwise the reason the stream ends there.
    fn word(&self, addr: u64) -> Result<u32, DisasmEnd> {
        let past_segment = DisasmEnd::SegmentEnd { addr };
        let offset = addr.checked_sub(self.segment.vaddr).ok_or(past_segment)?;
        let end = offset.checked_add(4).ok_or(past_segment)?;
        if end > self.segment.filesz {
            return Err(if end <= self.segment.memsz {
                DisasmEnd::ZeroFill { addr }
            } else {
                past_segment
            });
        }
        let at = self
            .segment
            .file_offset
            .checked_add(offset)
            .and_then(|at| usize::try_from(at).ok())
            .ok_or(past_segment)?;
        // A segment from `checked_pt_loads` holds its file bytes inside
        // the file; one whose bytes run past it ends here.
        let bytes = self.elf.get(at..at.checked_add(4).ok_or(past_segment)?);
        let bytes = bytes.ok_or(past_segment)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

impl<'a> Iterator for Disassembly<'a> {
    type Item = DisasmItem<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if let Some(item) = self.pending.take() {
            return Some(item);
        }
        let Some(addr) = self
            .words
            .checked_mul(4)
            .and_then(|delta| self.start.checked_add(delta))
        else {
            let words = self.words;
            return self.end(DisasmEnd::AddressOverflow { words });
        };
        let raw = match self.word(addr) {
            Ok(raw) => raw,
            Err(end) => return self.end(end),
        };
        self.words += 1;
        let item = match decode(raw) {
            Ok(insn) => DisasmItem::Instruction { addr, raw, insn },
            Err(_) => DisasmItem::Undecodable { addr, raw },
        };
        let starts = self
            .symbols
            .zip(u32::try_from(addr).ok())
            .and_then(|(map, addr32)| map.span_at(addr32).filter(|span| span.start == addr32));
        match starts {
            Some(span) => {
                self.pending = Some(item);
                Some(DisasmItem::FunctionStart(span))
            }
            None => Some(item),
        }
    }
}

#[cfg(test)]
#[path = "tests/disasm_tests.rs"]
mod tests;
