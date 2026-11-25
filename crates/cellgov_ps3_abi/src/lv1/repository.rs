//! The repository: the key-value tree lv1 keeps for each partition, the
//! encoding of its key words, and the nodes whose values a source gives.
//!
//! A read names a partition and four raw key words. Text fields pack
//! their bytes from the high end of the word, as the big-endian `memcpy`
//! in Linux's helpers does. An index adds to the word.

use super::partition::LPAR_ID_GAME_OS;

/// The partition and the four raw words of a repository read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryKey {
    /// The partition the read names, from r3.
    pub lpar: u64,
    /// The first key word, from r4.
    pub n1: u64,
    /// The second key word, from r5.
    pub n2: u64,
    /// The third key word, from r6.
    pub n3: u64,
    /// The fourth key word, from r7.
    pub n4: u64,
}

/// Up to 8 bytes of `text`, from the high byte of the word down.
const fn pack(text: &[u8]) -> u64 {
    let mut word = 0u64;
    let mut i = 0;
    while i < text.len() && i < 8 {
        word |= (text[i] as u64) << (56 - 8 * i);
        i += 1;
    }
    word
}

/// The first key word: up to 4 bytes of `text` in the low word, plus
/// `index`.
///
/// Source: Linux `arch/powerpc/platforms/ps3/repository.c`,
/// `make_first_field`, which also sets the vendor id to 0.
pub const fn first_field(text: &[u8], index: u64) -> u64 {
    (pack(text) >> 32).wrapping_add(index)
}

/// A later key word: up to 8 bytes of `text`, plus `index`.
///
/// Source: Linux `arch/powerpc/platforms/ps3/repository.c`,
/// `make_field`.
pub const fn field(text: &[u8], index: u64) -> u64 {
    pack(text).wrapping_add(index)
}

/// A repository node and its two values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepositoryNode {
    /// The read that names the node.
    pub key: RepositoryKey,
    /// The first value, which a successful read returns in r4.
    pub v1: u64,
    /// The second value, which a successful read returns in r5.
    pub v2: u64,
    /// The source of both values.
    pub source: &'static str,
}

/// The source of the nodes from psdevwiki's 3.41 GameOS dump.
const GAME_OS_341_DUMP: &str = "psdevwiki \"Repository Nodes\", section \"3.41 GameOS\" \
     (Wayback snapshot 2025-12-20)";

/// Every node whose values a source gives, ascending by key.
///
/// The psdevwiki dump records no partition. CellGov keys its nodes under
/// [`LPAR_ID_GAME_OS`], because the dump comes from GameOS.
pub const NODES: &[RepositoryNode] = &[
    // `bi.pu`, index 0: the logical PU id. Linux reads it in
    // `ps3_repository_read_pu_id`.
    RepositoryNode {
        key: RepositoryKey {
            lpar: LPAR_ID_GAME_OS,
            n1: first_field(b"bi", 0),
            n2: field(b"pu", 0),
            n3: 0,
            n4: 0,
        },
        v1: 1,
        v2: 0,
        source: GAME_OS_341_DUMP,
    },
    // `bi.pun`: the number of logical PUs in the partition. Linux reads it
    // in `ps3_repository_read_num_pu`.
    RepositoryNode {
        key: RepositoryKey {
            lpar: LPAR_ID_GAME_OS,
            n1: first_field(b"bi", 0),
            n2: field(b"pun", 0),
            n3: 0,
            n4: 0,
        },
        v1: 1,
        v2: 0,
        source: GAME_OS_341_DUMP,
    },
];

/// The node `key` names, or `None` when no source gives its values.
pub fn lookup(key: &RepositoryKey) -> Option<&'static RepositoryNode> {
    NODES
        .binary_search_by(|node| node.key.cmp(key))
        .ok()
        .map(|at| &NODES[at])
}

#[cfg(test)]
#[path = "tests/repository_tests.rs"]
mod tests;
