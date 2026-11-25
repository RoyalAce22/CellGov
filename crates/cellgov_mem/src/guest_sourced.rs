//! Sourced words: words of an unmodelled span whose value a source gives
//! for the loads of one instruction.
//!
//! A sourced word adds no bytes to the span. An execution unit asks
//! [`GuestMemory::sourced_word`] for the word only at a load that stops at
//! the span, and only for the reader that the source names. Every other
//! access stops at the span. A source can give only some bits of a word.
//! The loader then declares the word for a reader whose code keeps only
//! those bits.
//!
//! A sourced word is not guest state: only a loader declares one, and no
//! store changes one. So the content hash leaves the sourced words out.

use super::{GuestMemory, MemError};

/// The bytes of one word of an unmodelled span, for the loads of one
/// instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourcedWord {
    /// The real address of the word's first byte.
    real: u64,
    /// The effective address of the instruction that the word answers.
    reader: u64,
    /// The word, big-endian.
    bytes: [u8; 4],
}

impl GuestMemory {
    /// Record `value` as the word at `real` for the loads of the
    /// instruction at the effective address `reader`.
    ///
    /// # Errors
    ///
    /// Records nothing on an error. Returns one of these:
    /// - [`MemError::SourcedWordUnheld`] when `real` is not a multiple of
    ///   4, or when one unmodelled span does not hold all 4 bytes of the
    ///   word;
    /// - [`MemError::SourcedWordRepeated`] when the memory holds a word at
    ///   `real` for `reader` already;
    /// - [`MemError::SourcedWordRegistered`] when the memory holds a
    ///   register word at `real`.
    pub fn declare_sourced_word(
        &mut self,
        real: u64,
        value: u32,
        reader: u64,
    ) -> Result<(), MemError> {
        let held = real.is_multiple_of(4)
            && self
                .unmodelled_spans
                .iter()
                .any(|&(lo, hi)| lo <= real && real.checked_add(4).is_some_and(|end| end <= hi));
        if !held {
            return Err(MemError::SourcedWordUnheld { real, reader });
        }
        let key = |w: &SourcedWord| (w.real, w.reader);
        let at = match self
            .sourced_words
            .binary_search_by_key(&(real, reader), key)
        {
            Ok(_) => return Err(MemError::SourcedWordRepeated { real, reader }),
            Err(at) => at,
        };
        if self.register_word(real).is_some() {
            return Err(MemError::SourcedWordRegistered { real, reader });
        }
        let word = SourcedWord {
            real,
            reader,
            bytes: value.to_be_bytes(),
        };
        self.sourced_words.insert(at, word);
        Ok(())
    }

    /// The word that holds the byte at `real` for the instruction at
    /// `reader`: the word's real address and its bytes. Returns `None`
    /// when the memory holds no such word.
    pub fn sourced_word(&self, real: u64, reader: u64) -> Option<(u64, &[u8])> {
        let word_real = real & !3;
        self.sourced_words
            .binary_search_by_key(&(word_real, reader), |w| (w.real, w.reader))
            .ok()
            .map(|i| (word_real, &self.sourced_words[i].bytes[..]))
    }

    /// Whether the memory holds a sourced word at `real` for any reader.
    pub(super) fn holds_sourced_word(&self, real: u64) -> bool {
        let first = self.sourced_words.partition_point(|w| w.real < real);
        self.sourced_words
            .get(first)
            .is_some_and(|w| w.real == real)
    }
}
