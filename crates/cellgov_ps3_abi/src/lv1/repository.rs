//! The repository: the key-value tree lv1 keeps for each partition, the
//! encoding of its key words, and the nodes whose values a source gives.
//!
//! A read names a partition and four raw key words. Text fields pack
//! their bytes from the high end of the word, as the big-endian `memcpy`
//! in Linux's helpers does. An index adds to the word.

use super::partition::{LPAR_ID_GAME_OS, LPAR_ID_PME};

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
    ///   low bytes after the name hold, when that index is not 0;
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
    /// A console capture that read the first value back from the running
    /// kernel's memory.
    ///
    /// The kernel keeps only the first value of the read, so the capture
    /// does not observe the second: see [`SecondValue::Unobserved`].
    ConsoleCapture {
        /// The capture directory, relative to the workspace root.
        capture: &'static str,
        /// The kernel address whose doubleword the capture read.
        address: u64,
    },
    /// A console capture that read both values from the node in lv1's
    /// repository, through a read-only peek of lv1 memory.
    Lv1NodeCapture {
        /// The capture directory, relative to the workspace root.
        capture: &'static str,
        /// The lv1 address of the node's first value on the captured
        /// console.
        address: u64,
    },
}

impl NodeSource {
    /// The class of the source: `psdevwiki` or `console`.
    pub fn class(&self) -> &'static str {
        match self {
            Self::PsdevwikiDump { .. } => "psdevwiki",
            Self::ConsoleCapture { .. } | Self::Lv1NodeCapture { .. } => "console",
        }
    }

    /// The locator of the source: the citation alias of a psdevwiki page,
    /// or the directory of a console capture.
    pub fn citation(&self) -> &'static str {
        match self {
            Self::PsdevwikiDump { alias, .. } => alias,
            Self::ConsoleCapture { capture, .. } | Self::Lv1NodeCapture { capture, .. } => capture,
        }
    }

    /// The part of the source that holds the value: the page section of a
    /// psdevwiki dump, or the address a console capture read, as 16 hex
    /// digits.
    pub fn section(&self) -> Option<String> {
        match self {
            Self::PsdevwikiDump { section, .. } => Some((*section).to_string()),
            Self::ConsoleCapture { address, .. } | Self::Lv1NodeCapture { address, .. } => {
                Some(format!("{address:#018x}"))
            }
        }
    }
}

/// A place in a kernel that makes a repository read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadSite {
    /// The address of the `sc`.
    pub sc: u64,
    /// The link register at the `sc`: the return address into the
    /// function that called the read wrapper.
    pub lr: u64,
}

/// The second value of a node, which a successful read returns in r5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondValue {
    /// The source gives the value.
    Given(u64),
    /// The source does not observe the value.
    Unobserved {
        /// The read sites whose code does not read the second value
        /// back. A static analysis of the kernel shows this for each site.
        discarded_by: &'static [ReadSite],
    },
}

/// A repository node, its two values and the firmware CellGov serves them to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepositoryNode {
    /// The read that names the node.
    pub key: RepositoryKey,
    /// The first value, which a successful read returns in r4.
    pub v1: u64,
    /// The second value, which a successful read returns in r5.
    pub v2: SecondValue,
    /// The firmware versions CellGov serves the values to, ascending, such
    /// as `"4.93"`. A read at any other firmware does not match the node.
    /// The values can come from a source of another firmware: the
    /// psdevwiki dump is of 3.41.
    pub firmware: &'static [&'static str],
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

/// The firmware whose kernel boots CellGov serves the psdevwiki dump's
/// values to.
///
/// The dump is of firmware 3.41, which no logged boot is. No source gives
/// the values for 1.50, 1.94 or 3.70. At 4.93 the console captures
/// [`BOOT_RECORD_CAPTURE`], [`DEBUG_SUPPORT_CAPTURE`],
/// [`FLASH_EXT_CAPTURE`] and [`BUS_CAPTURE`] give each value that the 4.93
/// boot reads.
const DUMP_FIRMWARE_BEFORE_4_93: &[&str] = &["1.50", "1.94", "3.70"];

/// The console capture of the 4.93 kernel's boot repository record, on
/// the reference console.
///
/// The kernel stores the values of its boot repository reads in one
/// record at [`BOOT_RECORD_4_93`]
/// [OFW-4.93 lv2_kernel a:0x800000000000CBD4-0x800000000000CD4C]. The
/// capture read each doubleword back through the CFW's lv2 peek.
pub const BOOT_RECORD_CAPTURE: &str = "tests/micro/lv2_repository_boot_nodes/ps3/cech20-cex-493";

/// The address of the 4.93 kernel's boot repository record.
pub const BOOT_RECORD_4_93: u64 = 0x8000_0000_0037_a5a0;

/// The firmware of [`BOOT_RECORD_CAPTURE`], [`DEBUG_SUPPORT_CAPTURE`],
/// [`FLASH_EXT_CAPTURE`] and [`BUS_CAPTURE`].
const CAPTURE_FIRMWARE: &[&str] = &["4.93"];

/// The value at offset `offset` of the 4.93 boot record, as
/// [`BOOT_RECORD_CAPTURE`] read it.
const fn captured(offset: u64) -> NodeSource {
    NodeSource::ConsoleCapture {
        capture: BOOT_RECORD_CAPTURE,
        address: BOOT_RECORD_4_93 + offset,
    }
}

/// The `sc` of the 4.93 kernel's repository read wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000000CE3C]. Every boot repository read
/// of the 4.93 kernel goes through it.
///
/// The wrapper stores the first value through its third argument and the
/// second through its fourth. The README of [`BOOT_RECORD_CAPTURE`] gives
/// the analysis.
pub const READ_WRAPPER_SC_4_93: u64 = 0x8000_0000_0000_ce3c;

/// The boot read whose wrapper call returns to `lr` in the 4.93 kernel.
///
/// Each such caller is a leaf accessor. It passes a stack slot as the
/// wrapper's fourth argument and does not load that slot again.
const fn boot_read_4_93(lr: u64) -> ReadSite {
    ReadSite {
        sc: READ_WRAPPER_SC_4_93,
        lr,
    }
}

/// The console capture of lpar 1's `ss.param.debug.support` node.
///
/// The capture reads the node in lv1's repository, on the reference
/// console at firmware 4.93. Its README gives the method.
pub const DEBUG_SUPPORT_CAPTURE: &str =
    "tests/micro/lv1_repository_debug_support/peek/cech20-cex-493";

/// The lv1 address of the first value of the node that
/// [`DEBUG_SUPPORT_CAPTURE`] read.
const DEBUG_SUPPORT_V1_ADDRESS: u64 = 0x0076_3d58;

/// The console capture of lpar 1's `sys.flash.ext` node in lv1's
/// repository, on the reference console at firmware 4.93. Its README
/// gives the method.
pub const FLASH_EXT_CAPTURE: &str = "tests/micro/lv1_repository_flash/peek/cech20-cex-493";

/// The lv1 address of the first value of the node that
/// [`FLASH_EXT_CAPTURE`] read.
const FLASH_EXT_V1_ADDRESS: u64 = 0x0068_8d78;

/// The console capture of lpar 1's bus nodes in lv1's repository, on the
/// reference console at firmware 4.93. Its README gives the method.
pub const BUS_CAPTURE: &str = "tests/micro/lv1_repository_bus/peek/cech20-cex-493";

/// A node of lpar 1's bus `bus` whose last three key words are `n`.
///
/// [`BUS_CAPTURE`] read the node's two values at the lv1 addresses
/// `address` and `address` + 8.
const fn bus_node(bus: u64, n: [u64; 3], v1: u64, v2: u64, address: u64) -> RepositoryNode {
    node(
        LPAR_ID_PME,
        [first_field(b"bus", bus), n[0], n[1], n[2]],
        v1,
        SecondValue::Given(v2),
        CAPTURE_FIRMWARE,
        NodeSource::Lv1NodeCapture {
            capture: BUS_CAPTURE,
            address,
        },
    )
}

/// The second value of a node that [`BOOT_RECORD_CAPTURE`] gives.
///
/// The capture does not observe it, and the reads at `discarded_by`
/// discard it.
const fn unobserved(discarded_by: &'static [ReadSite]) -> SecondValue {
    SecondValue::Unobserved { discarded_by }
}

const fn node(
    lpar: u64,
    n: [u64; 4],
    v1: u64,
    v2: SecondValue,
    firmware: &'static [&'static str],
    source: NodeSource,
) -> RepositoryNode {
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
        firmware,
        source,
    }
}

/// Every node whose values a source gives, ascending by key and then by
/// firmware.
///
/// The table holds only the nodes that a logged kernel boot reads, at
/// the firmware whose boot reads them. Two nodes with one key name no
/// firmware in common.
///
/// The psdevwiki dump records no partition. CellGov keys a `bi` node
/// under [`LPAR_ID_GAME_OS`], the partition the dump comes from and the
/// partition the kernel names.
pub const NODES: &[RepositoryNode] = &[
    // `be.0.clock`: the time base frequency. The kernel reads it with a 0
    // second word and keeps it at offset 0xb0 of its boot record
    // [OFW-4.93 lv2_kernel a:0x800000000000CD14].
    node(
        LPAR_ID_PME,
        [first_field(b"be", 0), 0, field(b"clock", 0), 0],
        0x04c1_a6c0,
        unobserved(&[boot_read_4_93(0x8000_0000_0000_c85c)]),
        CAPTURE_FIRMWARE,
        captured(0xb0),
    ),
    // `bus.<n>.type` for n = 0, 1: the bus type. The kernel's bus search
    // reads it for n = 0 to 3 and stops at the first bus of type 4
    // [OFW-4.93 lv2_kernel a:0x80000000002777f8-0x8000000000277830]. Its
    // caller asks for type 4 [OFW-4.93 lv2_kernel a:0x8000000000277910].
    // Bus 1 is the first bus of type 4 on the reference console.
    bus_node(0, [field(b"type", 0), 0, 0], 3, 0, 0x0068_9ff8),
    // `bus.1.dev.<n>.id`: the device id. The handler of a device of type 3
    // reads it and gives it to the device object with the bus id
    // [OFW-4.93 lv2_kernel a:0x80000000000936cc-0x80000000000936ec].
    bus_node(1, [field(b"dev", 0), field(b"id", 0), 0], 0, 0, 0x0068_a1f8),
    // `bus.1.dev.<n>.type`: the device type. The kernel reads it for each
    // device of the bus and selects a handler for types 3, 4 and 6
    // [OFW-4.93 lv2_kernel a:0x80000000000937fc-0x8000000000093844].
    bus_node(
        1,
        [field(b"dev", 0), field(b"type", 0), 0],
        3,
        0,
        0x0068_a278,
    ),
    // `bus.1.id`: the bus id. The bus search reads it for the bus it finds
    // [OFW-4.93 lv2_kernel a:0x800000000027786c]. The kernel reads it again
    // and keeps the low word of v2 when v2 is not 0
    // [OFW-4.93 lv2_kernel a:0x8000000000093900-0x8000000000093924].
    bus_node(1, [field(b"id", 0), 0, 0], 1, 0x0400_0102, 0x0068_acf8),
    // `bus.1.num_dev`: the number of devices on the bus. The bus search
    // reads it [OFW-4.93 lv2_kernel a:0x8000000000277880], and the device
    // loop runs that many times
    // [OFW-4.93 lv2_kernel a:0x80000000000937bc-0x80000000000937f8].
    bus_node(1, [field(b"num_dev", 0), 0, 0], 4, 0, 0x0068_adf8),
    bus_node(1, [field(b"type", 0), 0, 0], 4, 0, 0x0068_ad78),
    // `plat.id`: the platform id, 8 bytes of text. The kernel keeps it at
    // offset 0xa8 of its boot record [OFW-4.93 lv2_kernel a:0x800000000000CD08].
    node(
        LPAR_ID_PME,
        [first_field(b"plat", 0), field(b"id", 0), 0, 0],
        0x436f_6b47_3131_0000,
        unobserved(&[boot_read_4_93(0x8000_0000_0000_c8d0)]),
        CAPTURE_FIRMWARE,
        captured(0xa8),
    ),
    // `ss.param.debug.support`. The kernel stores both values
    // [OFW-4.93 lv2_kernel a:0x8000000000050a70-0x8000000000050a80].
    // lpar 1's update manager sets both to all-ones unless the unit's
    // target id is 0x81 or 0x82 [OFW-4.93 lv1 a:0x4F4874-0x4F48E4]. The
    // reference console is a retail unit.
    node(
        LPAR_ID_PME,
        [
            first_field(b"ss", 0),
            field(b"param", 0),
            field(b"debug", 0),
            field(b"support", 0),
        ],
        0xffff_ffff_ffff_ffff,
        SecondValue::Given(0xffff_ffff_ffff_ffff),
        CAPTURE_FIRMWARE,
        NodeSource::Lv1NodeCapture {
            capture: DEBUG_SUPPORT_CAPTURE,
            address: DEBUG_SUPPORT_V1_ADDRESS,
        },
    ),
    // `sys.flash.ext`. The kernel keeps only r4, the first value. Its reader returns 1 for
    // a status 0 answer only when bit 1 of r4 is clear
    // [OFW-4.93 lv2_kernel a:0x8000000000050e18-0x8000000000050e48]. Only
    // a return of 1 leads to the `sys.flash.boot` read
    // [OFW-4.93 lv2_kernel a:0x800000000004f23c-0x800000000004f24c]. On
    // the reference console, bit 1 of v1 is set.
    node(
        LPAR_ID_PME,
        [
            first_field(b"sys", 0),
            field(b"flash", 0),
            field(b"ext", 0),
            0,
        ],
        0xfe,
        SecondValue::Given(0),
        CAPTURE_FIRMWARE,
        NodeSource::Lv1NodeCapture {
            capture: FLASH_EXT_CAPTURE,
            address: FLASH_EXT_V1_ADDRESS,
        },
    ),
    // `bi.pu` at index 0: the logical PU id. Linux reads it in
    // `ps3_repository_read_pu_id`.
    node(
        LPAR_ID_GAME_OS,
        [first_field(b"bi", 0), field(b"pu", 0), 0, 0],
        1,
        SecondValue::Given(0),
        DUMP_FIRMWARE_BEFORE_4_93,
        GAME_OS_341_DUMP,
    ),
    // The same node at 4.93: the kernel keeps it at offset 0x18 of its boot
    // record [OFW-4.93 lv2_kernel a:0x800000000000CC78].
    node(
        LPAR_ID_GAME_OS,
        [first_field(b"bi", 0), field(b"pu", 0), 0, 0],
        1,
        unobserved(&[boot_read_4_93(0x8000_0000_0000_caf8)]),
        CAPTURE_FIRMWARE,
        captured(0x18),
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
        SecondValue::Given(0),
        DUMP_FIRMWARE_BEFORE_4_93,
        GAME_OS_341_DUMP,
    ),
    // The same node at 4.93: the kernel keeps it at offset 0x30 of its boot
    // record [OFW-4.93 lv2_kernel a:0x800000000000CC64-0x800000000000CC80].
    node(
        LPAR_ID_GAME_OS,
        [
            first_field(b"bi", 0),
            field(b"pu", 0),
            1,
            field(b"rm_addr", 0),
        ],
        0,
        unobserved(&[boot_read_4_93(0x8000_0000_0000_ca0c)]),
        CAPTURE_FIRMWARE,
        captured(0x30),
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
        SecondValue::Given(0),
        DUMP_FIRMWARE_BEFORE_4_93,
        GAME_OS_341_DUMP,
    ),
    // The same node at 4.93: the kernel keeps it at offset 0x38 of its boot
    // record [OFW-4.93 lv2_kernel a:0x800000000000CC64-0x800000000000CC80].
    node(
        LPAR_ID_GAME_OS,
        [
            first_field(b"bi", 0),
            field(b"pu", 0),
            1,
            field(b"rm_size", 0),
        ],
        0x0800_0000,
        unobserved(&[boot_read_4_93(0x8000_0000_0000_ca38)]),
        CAPTURE_FIRMWARE,
        captured(0x38),
    ),
    // `bi.pun`: the number of logical PUs in the partition. Linux reads it
    // in `ps3_repository_read_num_pu`.
    node(
        LPAR_ID_GAME_OS,
        [first_field(b"bi", 0), field(b"pun", 0), 0, 0],
        1,
        SecondValue::Given(0),
        DUMP_FIRMWARE_BEFORE_4_93,
        GAME_OS_341_DUMP,
    ),
    // The same node at 4.93: the kernel keeps it at offset 0x10 of its boot
    // record [OFW-4.93 lv2_kernel a:0x800000000000CC1C].
    node(
        LPAR_ID_GAME_OS,
        [first_field(b"bi", 0), field(b"pun", 0), 0, 0],
        1,
        unobserved(&[boot_read_4_93(0x8000_0000_0000_cba4)]),
        CAPTURE_FIRMWARE,
        captured(0x10),
    ),
    // `bi.rgntotal`: the total size of the partition's memory regions. The
    // kernel keeps it at offset 0x68 of its boot record
    // [OFW-4.93 lv2_kernel a:0x800000000000CD00].
    node(
        LPAR_ID_GAME_OS,
        [first_field(b"bi", 0), field(b"rgntotal", 0), 0, 0],
        0x0f9a_0000,
        unobserved(&[boot_read_4_93(0x8000_0000_0000_c94c)]),
        CAPTURE_FIRMWARE,
        captured(0x68),
    ),
];

/// The node `key` names at the firmware version `firmware`, or `None`
/// when no source gives its values for that firmware.
pub fn lookup(key: &RepositoryKey, firmware: &str) -> Option<&'static RepositoryNode> {
    NODES
        .iter()
        .find(|node| node.key == *key && node.firmware.contains(&firmware))
}

#[cfg(test)]
#[path = "tests/repository_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/repository_path_tests.rs"]
mod path_tests;

#[cfg(test)]
#[path = "tests/repository_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/repository_second_value_tests.rs"]
mod second_value_tests;

#[cfg(test)]
#[path = "tests/repository_debug_support_tests.rs"]
mod debug_support_tests;

#[cfg(test)]
#[path = "tests/repository_flash_ext_tests.rs"]
mod flash_ext_tests;

#[cfg(test)]
#[path = "tests/repository_bus_tests.rs"]
mod bus_tests;
