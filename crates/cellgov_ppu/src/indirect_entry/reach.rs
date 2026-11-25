//! The words of the code that control can reach from the entries of the
//! bound.
//!
//! Data in a code segment, such as a string or a table, decodes as
//! instructions. Control does not reach a word of data at run time. So
//! the constants, the CTR and LR values and the switches of its decoded
//! form are not values of the code. The bound takes these only from the
//! words of this set.
//!
//! A word is in the set when control can get to it from a start by
//! these steps, which the hypercall walk takes backward:
//! - a fall-through: from each word except a `b`, `bc`, `bclr` or `bcctr`
//!   that branches always and does not link. A call falls through to the
//!   word where its routine returns;
//! - a direct branch, to its target.
//!
//! The starts are these:
//! - each word that the bound records as an entry: the ELF entry, an
//!   interrupt vector, a named word, a target of a CTR or LR write, or a
//!   word where a switch puts control;
//! - each value that a load of 1, 2, 4 or 8 bytes at any address of the
//!   file bytes gives, in either byte order, with zero and with sign
//!   extension. Such a value can name a code address that the bound does
//!   not record yet, for example at an address that is not word-aligned.
//!   These values add no word to the bound;
//! - every word, when a `bcctr`, `bclr` or `rfid` that the bound lost can
//!   enter any word.
//!
//! For each start and each step, the set holds the word at the address
//! with bits 62 and 63 at 0 and the word at its low-order 32 bits. It
//! also holds each word at the real address of these two. So it holds
//! each word that control can get to in real mode or in 32-bit mode,
//! also after a switch to 32-bit mode or with MSR\[SF\] at 0 in real
//! mode.
//!
//! [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
//! [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
//! [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
//! [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.

use super::ctr_flow::positions;
use crate::hypercall_cfg::{branch_target, low_word, Code};
use crate::instruction::PpuInstruction;
use crate::local_cfg::falls_through;

/// The words of the code that control can reach from the starts that
/// [`Self::grow`] takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Reach<'a> {
    code: &'a Code,
    /// Whether control can reach each word, in one vector for each code
    /// segment.
    reached: Vec<Vec<bool>>,
    /// Whether a start enters each word, in one vector for each code
    /// segment.
    started: Vec<Vec<bool>>,
    /// The number of words in `reached` that control can reach.
    count: usize,
    /// The number of words in `started` that a start enters.
    starts: usize,
    /// The number of words of the code.
    words: usize,
}

impl<'a> Reach<'a> {
    /// The set with no word of `code`.
    pub(super) fn new(code: &'a Code) -> Self {
        let reached: Vec<Vec<bool>> = code
            .segments()
            .iter()
            .map(|segment| vec![false; segment.words.len()])
            .collect();
        let words = reached.iter().map(Vec::len).sum();
        Self {
            code,
            started: reached.clone(),
            reached,
            count: 0,
            starts: 0,
            words,
        }
    }

    /// The set with every word of `code`, and with no word that a start
    /// enters.
    #[cfg(test)]
    pub(super) fn every(code: &'a Code) -> Self {
        let mut reach = Self::new(code);
        for words in &mut reach.reached {
            words.fill(true);
        }
        reach.count = reach.words;
        reach
    }

    /// Whether control can reach the word `word` of the segment at
    /// `index`.
    pub(super) fn contains(&self, index: usize, word: usize) -> bool {
        self.reached
            .get(index)
            .and_then(|words| words.get(word))
            .copied()
            .unwrap_or(false)
    }

    /// Whether a start enters the word `word` of the segment at `index`.
    ///
    /// A path that a start begins holds a CTR that no word of the scan
    /// writes.
    pub(super) fn started(&self, index: usize, word: usize) -> bool {
        self.started
            .get(index)
            .and_then(|words| words.get(word))
            .copied()
            .unwrap_or(false)
    }

    /// Adds every word, as a word that a start enters, and returns
    /// whether the set or its starts grew.
    pub(super) fn fill(&mut self) -> bool {
        if self.count == self.words && self.starts == self.words {
            return false;
        }
        for words in self.reached.iter_mut().chain(&mut self.started) {
            words.fill(true);
        }
        self.count = self.words;
        self.starts = self.words;
        true
    }

    /// Adds each word that control can reach from an address of
    /// `starts`, and returns whether the set or its starts grew.
    ///
    /// The walk ends the steps from one start before it takes the next
    /// start, so a long run of starts keeps only one walk in memory.
    pub(super) fn grow(&mut self, starts: impl IntoIterator<Item = u64>) -> bool {
        let before = (self.count, self.starts);
        let mut pending = Vec::new();
        for start in starts {
            for address in spread(self.code, start) {
                self.start(address);
                pending.push(address);
            }
            self.walk(&mut pending);
        }
        (self.count, self.starts) != before
    }

    /// Records each word at `address` as a word that a start enters.
    fn start(&mut self, address: u64) {
        for (index, word) in positions(self.code, address) {
            let started = &mut self.started[index][word];
            if !*started {
                *started = true;
                self.starts += 1;
            }
        }
    }

    /// Adds each word that control can reach from an address of
    /// `pending`, and leaves `pending` empty.
    fn walk(&mut self, pending: &mut Vec<u64>) {
        let code = self.code;
        while let Some(at) = pending.pop() {
            for (index, word) in positions(code, at) {
                if self.reached[index][word] {
                    continue;
                }
                self.reached[index][word] = true;
                self.count += 1;
                let raw = code.segments()[index].words[word];
                pending.extend(
                    steps(at, raw)
                        .into_iter()
                        .flatten()
                        .flat_map(|next| spread(code, next)),
                );
            }
        }
    }
}

/// The addresses that control can get to from the word `raw` at
/// `address`: the next word, unless the word never falls through, and
/// the target of a direct branch.
fn steps(address: u64, raw: u32) -> [Option<u64>; 2] {
    let next = address.checked_add(4);
    let (falls, target) = match crate::decode::decode(raw) {
        Ok(PpuInstruction::B { offset, aa, link }) => {
            (link, Some(branch_target(address, i64::from(offset), aa)))
        }
        Ok(PpuInstruction::Bc {
            bo,
            offset,
            aa,
            link,
            ..
        }) => (
            falls_through(bo, link),
            Some(branch_target(address, i64::from(offset), aa)),
        ),
        Ok(PpuInstruction::Bclr { bo, link, .. } | PpuInstruction::Bcctr { bo, link, .. }) => {
            (falls_through(bo, link), None)
        }
        _ => (true, None),
    };
    [next.filter(|_| falls), target]
}

/// The addresses of the words of `code` that control can enter at
/// `address`:
/// - the word with bits 62 and 63 at 0, and each word at its real
///   address;
/// - the word at its low-order 32 bits, and each word at the real address
///   of that word.
///
/// The list can hold a word more than once.
fn spread(code: &Code, address: u64) -> impl Iterator<Item = u64> + '_ {
    code.real_aliases(address & !0b11)
        .chain(code.real_aliases(low_word(address)))
}
