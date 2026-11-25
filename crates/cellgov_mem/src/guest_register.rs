//! Modelled registers: words of an unmodelled span that hold the state of
//! a device's registers.
//!
//! A loader declares the words of one device as a register file, with the
//! rule that applies a store to them. A word holds a value and the bits of
//! the value that are known. Each bit outside the known bits reads as 0.
//!
//! A load of a word gets the word only for an instruction that the loader
//! declares as a reader of it, and only while the bits that the reader
//! keeps are known. Every other load stops at the span.
//!
//! A store of a whole aligned word commits through the commit pipeline.
//! The memory gives the rule the stored value and the bits of it that the
//! storing code sets itself ([`RegisterStore::own`]). The rule changes the
//! words or refuses the store. A refused store stops at the span.
//!
//! A reader can name the store of its code that writes the loaded word
//! back ([`RegisterPassBack`]). Only that store can pass an unknown bit
//! back. The commit pipeline does not know which instruction made a
//! store, so the unit that runs the store accepts it only when the
//! commit gives the rule the bits that this store owns
//! ([`GuestMemory::register_store_accepted`]).
//!
//! A loader can also declare the store instructions that write a word
//! ([`GuestMemory::declare_register_writer`]). The unit then accepts a
//! store to that word only from a declared writer.
//!
//! The words are guest state, so the content hash covers them.

use super::{GuestMemory, MemError};

/// The state of one register word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterWord {
    /// The word. Each bit outside `known` is 0.
    pub value: u32,
    /// The bits of the word that are known.
    pub known: u32,
}

impl RegisterWord {
    /// A word of which no bit is known.
    pub const UNKNOWN: Self = Self { value: 0, known: 0 };

    /// A word of which every bit is known.
    pub const fn known(value: u32) -> Self {
        Self {
            value,
            known: u32::MAX,
        }
    }
}

/// A store of a whole word to a register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterStore {
    /// The stored word.
    pub value: u32,
    /// The bits of the stored word that the storing code sets itself.
    ///
    /// A bit is the code's own when one of these holds:
    /// - the bit is known;
    /// - the bit is 1, because an unknown bit reads as 0;
    /// - no declared reader of the word stores the bit back unchanged
    ///   without keeping it. A reader that keeps the bit reads the word
    ///   only while the bit is known.
    pub own: u32,
}

/// The store of a reader's code that writes the loaded word back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterPassBack {
    /// The effective address of the store instruction.
    pub store: u64,
    /// The bits of the loaded word that the store writes back unchanged.
    pub bits: u32,
}

/// The rule of a register file: it applies `store` to the word at
/// `index` of `words`, in the order of the declaration. It returns
/// `false` when the device refuses the store.
///
/// The memory gives the rule a copy of the words and keeps the copy only
/// when the rule returns `true`. The commit pipeline calls the rule once
/// to validate a store and again to apply it, so the result must depend
/// only on the arguments.
pub type RegisterRule = fn(words: &mut [RegisterWord], index: usize, store: RegisterStore) -> bool;

/// The words of one device that one rule changes.
#[derive(Debug, Clone)]
pub(crate) struct RegisterFile {
    /// The real address of each word, in the order of the declaration.
    reals: Vec<u64>,
    /// The state of each word.
    words: Vec<RegisterWord>,
    /// The value of each word, big-endian, for the views of loads.
    bytes: Vec<[u8; 4]>,
    /// For each word, the unknown bits that a declared reader can store
    /// back unchanged.
    passed: Vec<u32>,
    /// The state of each word at the declaration.
    initial: Vec<RegisterWord>,
    rule: RegisterRule,
}

impl RegisterFile {
    fn set(&mut self, words: Vec<RegisterWord>) {
        self.bytes = words.iter().map(|w| w.value.to_be_bytes()).collect();
        self.words = words;
    }

    /// The store of `value` to the word at `index`, as the rule gets it.
    fn store(&self, index: usize, value: u32) -> RegisterStore {
        let word = self.words[index];
        let unknown_zeros = !word.known & !value;
        RegisterStore {
            value,
            own: !(unknown_zeros & self.passed[index]),
        }
    }

    /// The words after `value` is stored to the word at `index`, or `None`
    /// when the rule refuses the store.
    fn after_store(&self, index: usize, value: u32) -> Option<Vec<RegisterWord>> {
        let mut words = self.words.clone();
        (self.rule)(&mut words, index, self.store(index, value)).then_some(words)
    }
}

/// An instruction that a register word answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RegisterReader {
    /// The real address of the word.
    real: u64,
    /// The effective address of the instruction.
    reader: u64,
    /// The bits that must be known for the word to answer the load.
    kept: u32,
    /// The store that writes the loaded word back, with the bits it can
    /// pass back unknown.
    pass_back: Option<RegisterPassBack>,
}

/// A store instruction that a register word takes a store from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RegisterWriter {
    /// The real address of the word.
    real: u64,
    /// The effective address of the store instruction.
    store: u64,
}

impl GuestMemory {
    /// Declare `words`, each at its real address, as one register file
    /// whose stores `rule` applies.
    ///
    /// # Errors
    ///
    /// Returns [`MemError::RegisterRefused`] for the first word at which
    /// one of these holds, and declares nothing:
    /// - its address is not a multiple of 4;
    /// - one unmodelled span does not hold all 4 bytes of it;
    /// - the memory holds a register word at its address already;
    /// - its value has a bit outside its known bits.
    pub fn declare_registers(
        &mut self,
        words: &[(u64, RegisterWord)],
        rule: RegisterRule,
    ) -> Result<(), MemError> {
        for (i, &(real, word)) in words.iter().enumerate() {
            let repeated = words[..i].iter().any(|&(r, _)| r == real);
            let held = real.is_multiple_of(4)
                && self.unmodelled_spans.iter().any(|&(lo, hi)| {
                    lo <= real && real.checked_add(4).is_some_and(|end| end <= hi)
                });
            if !held
                || repeated
                || self.register_at(real).is_some()
                || word.value & !word.known != 0
            {
                return Err(MemError::RegisterRefused { real });
            }
        }
        let states: Vec<RegisterWord> = words.iter().map(|&(_, w)| w).collect();
        let mut file = RegisterFile {
            reals: words.iter().map(|&(r, _)| r).collect(),
            words: Vec::new(),
            bytes: Vec::new(),
            passed: vec![0; words.len()],
            initial: states.clone(),
            rule,
        };
        file.set(states);
        self.register_files.push(file);
        self.cached_hash.set(None);
        Ok(())
    }

    /// Declare the instruction at the effective address `reader` a reader
    /// of the register word at `real`.
    ///
    /// The word answers the reader's load while the bits of `kept` are
    /// known. `pass_back` names the store of the reader's code that writes
    /// the loaded word back, and the bits that it writes back unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`MemError::RegisterRefused`], and declares nothing, when
    /// the memory holds no register word at `real`, or holds `reader` as
    /// a reader of it already.
    pub fn declare_register_reader(
        &mut self,
        real: u64,
        reader: u64,
        kept: u32,
        pass_back: Option<RegisterPassBack>,
    ) -> Result<(), MemError> {
        let refused = MemError::RegisterRefused { real };
        let (file, index) = self.register_at(real).ok_or(refused)?;
        let key = |r: &RegisterReader| (r.real, r.reader);
        let at = match self
            .register_readers
            .binary_search_by_key(&(real, reader), key)
        {
            Ok(_) => return Err(refused),
            Err(at) => at,
        };
        // A bit the reader needs known never reads as a 0 it did not know.
        let pass_back = pass_back.map(|p| RegisterPassBack {
            store: p.store,
            bits: p.bits & !kept,
        });
        let reader = RegisterReader {
            real,
            reader,
            kept,
            pass_back,
        };
        self.register_readers.insert(at, reader);
        let passed = pass_back.map_or(0, |p| p.bits);
        self.register_files[file].passed[index] |= passed;
        Ok(())
    }

    /// Declare the store instruction at the effective address `store` a
    /// writer of the register word at `real`.
    ///
    /// After the first such declaration, the word takes a store only from
    /// a declared writer ([`Self::register_store_accepted`]). A word with
    /// no declared writer takes a store from any instruction.
    ///
    /// # Errors
    ///
    /// Returns [`MemError::RegisterRefused`], and declares nothing, when
    /// the memory holds no register word at `real`, or holds `store` as a
    /// writer of it already.
    pub fn declare_register_writer(&mut self, real: u64, store: u64) -> Result<(), MemError> {
        let refused = MemError::RegisterRefused { real };
        self.register_at(real).ok_or(refused)?;
        let writer = RegisterWriter { real, store };
        match self.register_writers.binary_search(&writer) {
            Ok(_) => Err(refused),
            Err(at) => {
                self.register_writers.insert(at, writer);
                Ok(())
            }
        }
    }

    /// Whether the register word at `real` takes a store from the
    /// instruction at `store`: the word has no declared writer, or
    /// `store` is one.
    fn writer_allowed(&self, real: u64, store: u64) -> bool {
        let writer = RegisterWriter { real, store };
        self.register_writers.binary_search(&writer).is_ok()
            || !self.register_writers.iter().any(|w| w.real == real)
    }

    /// The bits of the register word at `real` that the store at `store`
    /// can pass back unknown.
    fn passed_by(&self, real: u64, store: u64) -> u32 {
        self.register_readers
            .iter()
            .filter(|r| r.real == real)
            .filter_map(|r| r.pass_back)
            .filter(|p| p.store == store)
            .fold(0, |bits, p| bits | p.bits)
    }

    /// The file and the index in it of the register word at `real`.
    fn register_at(&self, real: u64) -> Option<(usize, usize)> {
        self.register_files
            .iter()
            .enumerate()
            .find_map(|(f, file)| {
                file.reals
                    .iter()
                    .position(|&r| r == real)
                    .map(|index| (f, index))
            })
    }

    /// The state of the register word at `real`, or `None` when the memory
    /// holds no register word there.
    pub fn register_word(&self, real: u64) -> Option<RegisterWord> {
        let (file, index) = self.register_at(real)?;
        Some(self.register_files[file].words[index])
    }

    /// The register word that holds the byte at `real` for the load of the
    /// instruction at `reader`: the word's real address and its bytes.
    ///
    /// Returns `None` when the memory holds no such word, `reader` is not
    /// a reader of it, or a bit that the reader keeps is not known.
    pub fn register_read(&self, real: u64, reader: u64) -> Option<(u64, &[u8])> {
        let word_real = real & !3;
        let (file, index) = self.register_at(word_real)?;
        let declared = self
            .register_readers
            .binary_search_by_key(&(word_real, reader), |r| (r.real, r.reader))
            .ok()?;
        let file = &self.register_files[file];
        let kept = self.register_readers[declared].kept;
        (kept & !file.words[index].known == 0).then(|| (word_real, &file.bytes[index][..]))
    }

    /// The bytes of the register word at `real`, or `None` when the memory
    /// holds no register word there.
    pub fn register_bytes(&self, real: u64) -> Option<&[u8]> {
        let (file, index) = self.register_at(real)?;
        Some(&self.register_files[file].bytes[index][..])
    }

    /// Whether the device accepts a store of `value` to the register word
    /// at `real` by the instruction at the effective address `store`.
    ///
    /// The commit gives the rule a 0 in an unknown bit as passed back when
    /// any reader's store passes that bit back. So the store is refused
    /// unless the bits that the commit takes as passed back are the bits
    /// that this instruction passes back. Then the rule judges at the
    /// commit the bits that this instruction owns.
    ///
    /// A word with a declared writer refuses a store from any other
    /// instruction ([`Self::declare_register_writer`]).
    pub fn register_store_accepted(&self, real: u64, value: u32, store: u64) -> bool {
        self.register_at(real).is_some_and(|(file, index)| {
            let file = &self.register_files[file];
            let unknown_zeros = !file.words[index].known & !value;
            let taken = unknown_zeros & file.passed[index];
            let passed = unknown_zeros & self.passed_by(real, store);
            taken == passed
                && self.writer_allowed(real, store)
                && file.after_store(index, value).is_some()
        })
    }

    /// Whether `[addr, addr + length)` is one register word.
    pub fn is_register(&self, addr: u64, length: u64) -> bool {
        length == 4 && self.register_at(addr).is_some()
    }

    /// Validate a store of `bytes` to the register word at `addr` without
    /// changing it. Returns `None` when the range is not one register
    /// word.
    pub(crate) fn validate_register_store(
        &self,
        addr: u64,
        bytes: &[u8],
    ) -> Option<Result<(), MemError>> {
        let (file, index) = self.register_at(addr).filter(|_| bytes.len() == 4)?;
        let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let refused = MemError::RegisterStoreRefused { real: addr };
        Some(
            self.register_files[file]
                .after_store(index, value)
                .map(|_| ())
                .ok_or(refused),
        )
    }

    /// Apply a store of `bytes` to the register word at `addr`. Returns
    /// `None`, and changes nothing, when the range is not one register
    /// word.
    pub(crate) fn apply_register_store(
        &mut self,
        addr: u64,
        bytes: &[u8],
    ) -> Option<Result<(), MemError>> {
        let (file, index) = self.register_at(addr).filter(|_| bytes.len() == 4)?;
        let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let Some(words) = self.register_files[file].after_store(index, value) else {
            return Some(Err(MemError::RegisterStoreRefused { real: addr }));
        };
        self.register_files[file].set(words);
        self.cached_hash.set(None);
        Some(Ok(()))
    }

    /// Feed each register word's address, value and known bits to
    /// `hasher`. A memory with no register file feeds nothing.
    pub(crate) fn hash_registers(&self, hasher: &mut crate::hash::Fnv1aHasher) {
        for file in &self.register_files {
            for (real, word) in file.reals.iter().zip(&file.words) {
                hasher.write(&real.to_le_bytes());
                hasher.write(&word.value.to_le_bytes());
                hasher.write(&word.known.to_le_bytes());
            }
        }
    }

    /// Return each register word to its state at the declaration.
    pub(crate) fn reset_registers(&mut self) {
        for file in &mut self.register_files {
            let initial = file.initial.clone();
            file.set(initial);
        }
    }
}
