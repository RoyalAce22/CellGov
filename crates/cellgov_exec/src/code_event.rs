//! The instruction-modification events a unit retires, in program order,
//! for the runtime's record of unsynchronized code fetches.
//!
//! [PPC-Book2 p:11 s:1.8] after a store into an instruction location,
//! running that location is boundedly undefined until the program runs
//! the instruction-modification sequence. The runtime runs the stored
//! instruction as soon as the store commits, so it records each fetch
//! that relies on that. The events reach it in program order, so the
//! record does not depend on where a batch ends.

/// One retired event of a unit that bears on code coherence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodeEvent {
    /// The unit fetched the instruction at `pc`.
    ///
    /// A unit may omit a fetch from the 128-byte cache line of the
    /// previous fetch when no other event came between them.
    Fetch {
        /// The guest address of the fetched word.
        pc: u64,
    },
    /// The unit stored `len` bytes at `addr`.
    Store {
        /// The guest address of the first byte.
        addr: u64,
        /// The store width in bytes.
        len: u8,
        /// The stored word, for a word-aligned 4-byte store.
        word: Option<u32>,
    },
    /// `dcbst` on the block that holds `addr`.
    Dcbst {
        /// The effective address the instruction named.
        addr: u64,
    },
    /// A heavyweight `sync`.
    Sync,
    /// `icbi` on the block that holds `addr`.
    Icbi {
        /// The effective address the instruction named.
        addr: u64,
    },
    /// `isync`.
    Isync,
    /// A context-synchronizing instruction other than `isync`, such as
    /// `sc` [PPC-Book3 p:4 s:1.6.1].
    ///
    /// [PPC-Book2 p:12 s:1.8] it ends the sequence only for a unit that
    /// runs code another unit modified; for its own stores a unit needs
    /// `isync`.
    ContextSync,
}
