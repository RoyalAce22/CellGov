//! A count of SPU instruction words by opcode-map row and decoder class.

use std::collections::BTreeSet;

use cellgov_ps3_abi::format::elf::PF_X;
use cellgov_ps3_abi::hw::spu_isa::{SpuOpcodeRow, SPU_OPCODE_MAP};

use crate::disasm::{SpuWord, SpuWordClass};
use crate::image::find_embedded_spu_elfs;

/// Words counted by the opcode-map row they select, and the words that
/// select none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuCensus {
    /// Words per row, indexed as the opcode map.
    per_row: Vec<u64>,
    /// Words that select no row.
    unassigned: u64,
}

/// One opcode-map row's line in a census.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuCensusRow {
    /// The row.
    pub row: &'static SpuOpcodeRow,
    /// How the decoder treats every word of the row.
    pub class: SpuWordClass,
    /// Words counted.
    pub words: u64,
}

impl SpuCensus {
    /// An empty census.
    pub fn new() -> Self {
        Self {
            per_row: vec![0; SPU_OPCODE_MAP.len()],
            unassigned: 0,
        }
    }

    /// Count one word.
    pub fn add(&mut self, raw: u32) {
        match SpuWord::of(raw).row {
            Some((index, _)) => self.per_row[index] += 1,
            None => self.unassigned += 1,
        }
    }

    /// Count every big-endian word of `bytes`; a trailing partial word
    /// is not counted.
    pub fn add_bytes(&mut self, bytes: &[u8]) {
        for word in bytes.chunks_exact(4) {
            self.add(u32::from_be_bytes([word[0], word[1], word[2], word[3]]));
        }
    }

    /// Count the executable-segment words of each SPU image in `bytes`
    /// whose FNV-1a hash `seen` does not hold yet, and add those hashes,
    /// so an image held by several files counts once. Returns the images
    /// and the words counted.
    pub fn add_new_images(&mut self, bytes: &[u8], seen: &mut BTreeSet<u64>) -> (usize, u64) {
        let mut images = 0;
        let mut words = 0;
        for image in find_embedded_spu_elfs(bytes) {
            let image_bytes = image.bytes(bytes);
            if !seen.insert(cellgov_mem::fnv1a(image_bytes)) {
                continue;
            }
            let before = self.words();
            for segment in image.elf.segments.iter().filter(|s| s.flags & PF_X != 0) {
                self.add_bytes(segment.bytes(image_bytes));
            }
            images += 1;
            words += self.words() - before;
        }
        (images, words)
    }

    /// Add every count of `other`.
    pub fn merge(&mut self, other: &Self) {
        for (mine, theirs) in self.per_row.iter_mut().zip(&other.per_row) {
            *mine += theirs;
        }
        self.unassigned += other.unassigned;
    }

    /// Every row with at least one word, in opcode-map order.
    pub fn rows(&self) -> impl Iterator<Item = SpuCensusRow> + '_ {
        SPU_OPCODE_MAP
            .iter()
            .zip(&self.per_row)
            .filter(|(_, &words)| words != 0)
            .map(|(row, &words)| SpuCensusRow {
                row,
                // The decoder treats a row's words alike, so the row's
                // canonical word stands for all of them.
                class: SpuWord::of(row.canonical_word()).class,
                words,
            })
    }

    /// Words that select no row.
    pub fn unassigned(&self) -> u64 {
        self.unassigned
    }

    /// Words counted in `class`.
    pub fn words_in(&self, class: SpuWordClass) -> u64 {
        if class == SpuWordClass::Unassigned {
            return self.unassigned;
        }
        self.rows()
            .filter(|row| row.class == class)
            .map(|row| row.words)
            .sum()
    }

    /// Every word counted.
    pub fn words(&self) -> u64 {
        self.per_row.iter().sum::<u64>() + self.unassigned
    }
}

impl Default for SpuCensus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "tests/census_tests.rs"]
mod tests;
