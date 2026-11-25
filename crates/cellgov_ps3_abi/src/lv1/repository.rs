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

impl RepositoryKey {
    /// The key words as text, joined by `.`, with the trailing zero words
    /// left out.
    ///
    /// A word renders as follows:
    /// - its name bytes from the high end, then `+` and the index that the
    ///   remaining low bytes hold, when that index is not 0;
    /// - `0` for a zero word;
    /// - `0x` and its hex digits, when its high byte is not a name byte.
    ///
    /// A name byte is an ASCII letter, a digit or `_`. A first word with a
    /// vendor id in its high half renders in hex. The raw words stay the
    /// key: two words can render alike, for example the name `0x1` and the
    /// number 1.
    pub fn path(&self) -> String {
        let first = if self.n1 >> 32 == 0 {
            word_text(&self.n1.to_be_bytes()[4..])
        } else {
            format!("{:#x}", self.n1)
        };
        let mut words = vec![self.n2, self.n3, self.n4];
        while words.last() == Some(&0) {
            words.pop();
        }
        std::iter::once(first)
            .chain(words.into_iter().map(|w| word_text(&w.to_be_bytes())))
            .collect::<Vec<_>>()
            .join(".")
    }
}

impl std::fmt::Display for RepositoryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lpar {} {}", self.lpar, self.path())
    }
}

/// One key word as text, from its big-endian `bytes`: see
/// [`RepositoryKey::path`].
fn word_text(bytes: &[u8]) -> String {
    let value = |part: &[u8]| part.iter().fold(0u64, |acc, b| acc << 8 | u64::from(*b));
    let word = value(bytes);
    if word == 0 {
        return "0".to_string();
    }
    let name_len = bytes
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
        .count();
    if name_len == 0 {
        return format!("{word:#x}");
    }
    let name: String = bytes[..name_len].iter().map(|b| char::from(*b)).collect();
    let index = value(&bytes[name_len..]);
    if index == 0 {
        name
    } else {
        format!("{name}+{index}")
    }
}

/// Where the two values of a node come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSource {
    /// A repository dump on a psdevwiki page.
    PsdevwikiDump {
        /// The page's citation alias. The workspace citation schema maps
        /// the alias to the page's one archived snapshot.
        alias: &'static str,
        /// The page section that holds the dump.
        section: &'static str,
    },
}

impl NodeSource {
    /// The class of the source: `psdevwiki`.
    pub fn class(&self) -> &'static str {
        match self {
            Self::PsdevwikiDump { .. } => "psdevwiki",
        }
    }

    /// The locator of the source: the citation alias of a psdevwiki page.
    pub fn citation(&self) -> &'static str {
        match self {
            Self::PsdevwikiDump { alias, .. } => alias,
        }
    }

    /// The part of the source that holds the value: the page section of a
    /// psdevwiki dump.
    pub fn section(&self) -> Option<&'static str> {
        match self {
            Self::PsdevwikiDump { section, .. } => Some(section),
        }
    }
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
    pub source: NodeSource,
}

/// The repository dump of psdevwiki's "Repository Nodes" page
/// [PS3DEVWIKI-REPO s:3.41_GameOS].
///
/// The dump lists each node as its four key words and its two values. It
/// records no partition.
const GAME_OS_341_DUMP: NodeSource = NodeSource::PsdevwikiDump {
    alias: "PS3DEVWIKI-REPO",
    section: "3.41_GameOS",
};

const fn node(lpar: u64, n: [u64; 4], v1: u64, v2: u64, source: NodeSource) -> RepositoryNode {
    RepositoryNode {
        key: RepositoryKey {
            lpar,
            n1: n[0],
            n2: n[1],
            n3: n[2],
            n4: n[3],
        },
        v1,
        v2,
        source,
    }
}

/// Every node whose values a source gives, ascending by key.
///
/// The table holds only the nodes that a logged kernel boot reads. Each
/// value applies to every firmware, so the table holds no node whose
/// value a source or a boot shows to differ by firmware.
///
/// The psdevwiki dump records no partition. CellGov keys a `bi` node
/// under [`LPAR_ID_GAME_OS`], the partition the dump comes from and the
/// partition the kernel names.
pub const NODES: &[RepositoryNode] = &[
    // `bi.pu` at index 0: the logical PU id. Linux reads it in
    // `ps3_repository_read_pu_id`.
    node(
        LPAR_ID_GAME_OS,
        [first_field(b"bi", 0), field(b"pu", 0), 0, 0],
        1,
        0,
        GAME_OS_341_DUMP,
    ),
    // `bi.pu.<id>.rm_addr` for PU id 1: the base of the real mode memory.
    // Linux `ps3_repository_read_mm_info` reads no node for it and sets
    // the base to 0.
    node(
        LPAR_ID_GAME_OS,
        [
            first_field(b"bi", 0),
            field(b"pu", 0),
            1,
            field(b"rm_addr", 0),
        ],
        0,
        0,
        GAME_OS_341_DUMP,
    ),
    // `bi.pu.<id>.rm_size` for PU id 1: the size of the real mode memory.
    // Linux reads it in `ps3_repository_read_rm_size`.
    node(
        LPAR_ID_GAME_OS,
        [
            first_field(b"bi", 0),
            field(b"pu", 0),
            1,
            field(b"rm_size", 0),
        ],
        0x0100_0000,
        0,
        GAME_OS_341_DUMP,
    ),
    // `bi.pun`: the number of logical PUs in the partition. Linux reads it
    // in `ps3_repository_read_num_pu`.
    node(
        LPAR_ID_GAME_OS,
        [first_field(b"bi", 0), field(b"pun", 0), 0, 0],
        1,
        0,
        GAME_OS_341_DUMP,
    ),
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

#[cfg(test)]
#[path = "tests/repository_path_tests.rs"]
mod path_tests;
