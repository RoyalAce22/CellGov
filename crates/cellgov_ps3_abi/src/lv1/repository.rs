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
/// [`FLASH_EXT_CAPTURE`], [`HW_CONFIG_CAPTURE`], [`LIST_CAPTURE`],
/// [`BUS_CAPTURE`] and [`STORAGE_BUS_CAPTURE`] give each value that the
/// 4.93 boot reads.
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
/// [`FLASH_EXT_CAPTURE`], [`HW_CONFIG_CAPTURE`], [`LIST_CAPTURE`],
/// [`BUS_CAPTURE`] and [`STORAGE_BUS_CAPTURE`].
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

/// The console capture of lpar 1's `sys.hw.config` node in lv1's
/// repository, on the reference console at firmware 4.93. Its README
/// gives the method.
pub const HW_CONFIG_CAPTURE: &str = "tests/micro/lv1_repository_hw_config/peek/cech20-cex-493";

/// The lv1 address of the first value of the node that
/// [`HW_CONFIG_CAPTURE`] read.
const HW_CONFIG_V1_ADDRESS: u64 = 0x0068_9678;

/// The console capture of the whole list of lpar 1's repository in lv1,
/// on the reference console at firmware 4.93. Its README gives the
/// method.
pub const LIST_CAPTURE: &str = "tests/micro/lv1_repository_list/peek/cech20-cex-493";

/// The lv1 address of the first value of `ss.param.update.status`, as
/// [`LIST_CAPTURE`] read it.
const UPDATE_STATUS_V1_ADDRESS: u64 = 0x0076_3e58;

/// The console capture of lpar 1's bus nodes in lv1's repository, on the
/// reference console at firmware 4.93. Its README gives the method.
pub const BUS_CAPTURE: &str = "tests/micro/lv1_repository_bus/peek/cech20-cex-493";

/// The console capture of lpar 1's bus 4 `id` and `type` nodes in lv1's
/// repository, on the reference console at firmware 4.93. Its README
/// gives the method.
pub const STORAGE_BUS_CAPTURE: &str = "tests/micro/lv1_repository_storage_bus/peek/cech20-cex-493";

/// A node of lpar 1's bus `bus` whose last three key words are `n`.
///
/// [`BUS_CAPTURE`] read the node's two values at the lv1 addresses
/// `address` and `address` + 8.
const fn bus_node(bus: u64, n: [u64; 3], v1: u64, v2: u64, address: u64) -> RepositoryNode {
    bus_node_of(BUS_CAPTURE, bus, n, v1, v2, address)
}

/// A node of lpar 1's bus `bus` whose last three key words are `n`.
///
/// `capture` read the node's two values at the lv1 addresses `address`
/// and `address` + 8.
const fn bus_node_of(
    capture: &'static str,
    bus: u64,
    n: [u64; 3],
    v1: u64,
    v2: u64,
    address: u64,
) -> RepositoryNode {
    node(
        LPAR_ID_PME,
        [first_field(b"bus", bus), n[0], n[1], n[2]],
        v1,
        SecondValue::Given(v2),
        CAPTURE_FIRMWARE,
        NodeSource::Lv1NodeCapture { capture, address },
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
    // `bus.<n>.id` for n = 0, 2, 3, 4: the bus id. The storage bus search
    // reads `bus.<n>.id` and then `bus.<n>.type` for n = 0 to 9. It stops
    // at the first bus of type 5
    // [OFW-4.93 lv2_kernel a:0x80000000002871e4-0x8000000000287228]. It
    // keeps v1 of the id
    // [OFW-4.93 lv2_kernel a:0x8000000000287254-0x800000000028725c]. Its
    // accessor drops v2: it gives a slot of its own frame for v2 and frees
    // the frame without a load of that slot
    // [OFW-4.93 lv2_kernel a:0x80000000000962f8-0x800000000009631c].
    bus_node(0, [field(b"id", 0), 0, 0], 0, 0, 0x0068_9f78),
    // `bus.<n>.type` for n = 0 to 4: the bus type. The kernel's bus search
    // reads it for n = 0 to 3 and stops at the first bus of type 4
    // [OFW-4.93 lv2_kernel a:0x80000000002777f8-0x8000000000277830]. Its
    // caller asks for type 4 [OFW-4.93 lv2_kernel a:0x8000000000277910].
    // Bus 1 is the first bus of type 4 on the reference console. The
    // storage bus search also reads it; see `bus.<n>.id` above.
    bus_node(0, [field(b"type", 0), 0, 0], 3, 0, 0x0068_9ff8),
    // `bus.1.dev.<n>.id`: the device id. The handler of a device of type 3
    // reads it and gives it to the device object with the bus id
    // [OFW-4.93 lv2_kernel a:0x80000000000936cc-0x80000000000936ec].
    bus_node(1, [field(b"dev", 0), field(b"id", 0), 0], 0, 0, 0x0068_a1f8),
    // `bus.1.dev.0.intr.0`: the first interrupt node of device 0. The
    // handler of a device of type 3 reads it after its open of the device
    // [OFW-4.93 lv2_kernel a:0x8000000000093738-0x8000000000093748]. When
    // v1 is 2, it keeps v2 in the device object
    // [OFW-4.93 lv2_kernel a:0x8000000000093754-0x8000000000093790].
    bus_node(
        1,
        [field(b"dev", 0), field(b"intr", 0), 0],
        2,
        0,
        0x0068_a2f8,
    ),
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
    // `bus.1.dev.1.id`: the handler of the bus's first device of type 4
    // reads it and gives it to the device object
    // [OFW-4.93 lv2_kernel a:0x8000000000093648-0x8000000000093664].
    bus_node(1, [field(b"dev", 1), field(b"id", 0), 0], 1, 0, 0x0068_a378),
    // `bus.1.dev.1.intr.<i>` for i = 0, 1: the interrupt nodes of device 1.
    // The handler of type 4 reads them until a read fails. It keeps v2 at
    // +128 of the device object for type 3 and at +136 for type 4
    // [OFW-4.93 lv2_kernel a:0x800000000009356c-0x80000000000935d4]
    // [OFW-4.93 lv2_kernel a:0x8000000000093624-0x800000000009362c].
    bus_node(
        1,
        [field(b"dev", 1), field(b"intr", 0), 0],
        3,
        0x10,
        0x0068_a478,
    ),
    bus_node(
        1,
        [field(b"dev", 1), field(b"intr", 1), 0],
        4,
        0x0a,
        0x0068_a4f8,
    ),
    // `bus.1.dev.1.reg.<r>.data` and `.type` for r = 0, 1: the register
    // regions of device 1. The handler of type 4 reads the type and then
    // the data until a type read fails
    // [OFW-4.93 lv2_kernel a:0x80000000000934d4-0x8000000000093568]. It
    // maps a region of type 3 as region 1 and a region of type 4 as
    // region 2, with v1 as the address and v2 as the size
    // [OFW-4.93 lv2_kernel a:0x8000000000093518-0x8000000000093538]
    // [OFW-4.93 lv2_kernel a:0x8000000000093600-0x800000000009360c].
    bus_node(
        1,
        [field(b"dev", 1), field(b"reg", 0), field(b"data", 0)],
        0x0301_0000,
        0x0001_0000,
        0x0068_a5f8,
    ),
    bus_node(
        1,
        [field(b"dev", 1), field(b"reg", 0), field(b"type", 0)],
        3,
        0,
        0x0068_a578,
    ),
    bus_node(
        1,
        [field(b"dev", 1), field(b"reg", 1), field(b"data", 0)],
        0x0381_0000,
        0x0001_0000,
        0x0068_a6f8,
    ),
    bus_node(
        1,
        [field(b"dev", 1), field(b"reg", 1), field(b"type", 0)],
        4,
        0,
        0x0068_a678,
    ),
    // `bus.1.dev.1.type`; see `bus.1.dev.<n>.type` above.
    bus_node(
        1,
        [field(b"dev", 1), field(b"type", 0), 0],
        4,
        0,
        0x0068_a3f8,
    ),
    // `bus.1.dev.2.id`: the handler of a later device of type 4 reads it
    // and gives it to its device object
    // [OFW-4.93 lv2_kernel a:0x8000000000093484-0x8000000000093490].
    bus_node(1, [field(b"dev", 2), field(b"id", 0), 0], 2, 0, 0x0068_a778),
    // The interrupt nodes and the register regions of device 2, which the
    // handler of type 4 reads as it reads those of device 1.
    bus_node(
        1,
        [field(b"dev", 2), field(b"intr", 0), 0],
        3,
        0x11,
        0x0068_a878,
    ),
    bus_node(
        1,
        [field(b"dev", 2), field(b"intr", 1), 0],
        4,
        0x0b,
        0x0068_a8f8,
    ),
    bus_node(
        1,
        [field(b"dev", 2), field(b"reg", 0), field(b"data", 0)],
        0x0302_0000,
        0x0001_0000,
        0x0068_a9f8,
    ),
    bus_node(
        1,
        [field(b"dev", 2), field(b"reg", 0), field(b"type", 0)],
        3,
        0,
        0x0068_a978,
    ),
    bus_node(
        1,
        [field(b"dev", 2), field(b"reg", 1), field(b"data", 0)],
        0x0382_0000,
        0x0001_0000,
        0x0068_aaf8,
    ),
    bus_node(
        1,
        [field(b"dev", 2), field(b"reg", 1), field(b"type", 0)],
        4,
        0,
        0x0068_aa78,
    ),
    // `bus.1.dev.2.type`; see `bus.1.dev.<n>.type` above.
    bus_node(
        1,
        [field(b"dev", 2), field(b"type", 0), 0],
        4,
        0,
        0x0068_a7f8,
    ),
    // `bus.1.dev.3.id` and `.intr.0`: the handler of type 6 reads the id
    // [OFW-4.93 lv2_kernel a:0x8000000000093318-0x8000000000093328], and
    // then interrupt 0. It keeps v2 at +128 of its device object when v1
    // is 5 [OFW-4.93 lv2_kernel a:0x8000000000093380-0x80000000000933ac].
    bus_node(1, [field(b"dev", 3), field(b"id", 0), 0], 3, 0, 0x0068_ab78),
    bus_node(
        1,
        [field(b"dev", 3), field(b"intr", 0), 0],
        5,
        0x2f,
        0x0068_ac78,
    ),
    // `bus.1.dev.3.type`; see `bus.1.dev.<n>.type` above.
    bus_node(
        1,
        [field(b"dev", 3), field(b"type", 0), 0],
        6,
        0,
        0x0068_abf8,
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
    // `bus.1.type`; see `bus.<n>.type` above.
    bus_node(1, [field(b"type", 0), 0, 0], 4, 0, 0x0068_ad78),
    // `bus.2.id`; see `bus.<n>.id` above.
    bus_node(2, [field(b"id", 0), 0, 0], 2, 0, 0x0068_ae78),
    // `bus.2.type`; see `bus.<n>.type` above.
    bus_node(2, [field(b"type", 0), 0, 0], 1, 0, 0x0068_aef8),
    // `bus.3.id`; see `bus.<n>.id` above.
    bus_node(3, [field(b"id", 0), 0, 0], 3, 0, 0x0068_aff8),
    // `bus.3.type`; see `bus.<n>.type` above.
    bus_node(3, [field(b"type", 0), 0, 0], 2, 0, 0x0068_b078),
    // `bus.4.id`; see `bus.<n>.id` above.
    bus_node_of(
        STORAGE_BUS_CAPTURE,
        4,
        [field(b"id", 0), 0, 0],
        4,
        0,
        0x0069_4278,
    ),
    // `bus.4.type`; see `bus.<n>.type` above. Bus 4 is the first bus of
    // type 5 on the reference console, so the storage bus search stops
    // there.
    bus_node_of(
        STORAGE_BUS_CAPTURE,
        4,
        [field(b"type", 0), 0, 0],
        5,
        0,
        0x0069_42f8,
    ),
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
    // `ss.param.update.status`. Its reader stores only the low byte of v1
    // and does not store v2
    // [OFW-4.93 lv2_kernel a:0x8000000000050b24-0x8000000000050b50]. Each
    // of the reader's six callers compares that byte with 0xFF, for
    // example the constructor that the 4.93 boot runs
    // [OFW-4.93 lv2_kernel a:0x80000000000a1f84-0x80000000000a1fa0].
    node(
        LPAR_ID_PME,
        [
            first_field(b"ss", 0),
            field(b"param", 0),
            field(b"update", 0),
            field(b"status", 0),
        ],
        0xff,
        SecondValue::Given(0),
        CAPTURE_FIRMWARE,
        NodeSource::Lv1NodeCapture {
            capture: LIST_CAPTURE,
            address: UPDATE_STATUS_V1_ADDRESS,
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
    // `sys.hw.config`. The 4.93 boot reads it in the constructor that
    // keeps v1 at +8 of its object
    // [OFW-4.93 lv2_kernel a:0x80000000000a1f3c-0x80000000000a1f7c]. A
    // second function reads it and keeps two bits of v1
    // [OFW-4.93 lv2_kernel a:0x8000000000287764-0x8000000000287774]. A
    // third reads it and tests only the sign of v1
    // [OFW-4.93 lv2_kernel a:0x80000000000b3ec0-0x80000000000b3ed0]. No
    // read site loads v2.
    node(
        LPAR_ID_PME,
        [
            first_field(b"sys", 0),
            field(b"hw", 0),
            field(b"config", 0),
            0,
        ],
        0x4e00_ffff_0203_bc3c,
        SecondValue::Given(0),
        CAPTURE_FIRMWARE,
        NodeSource::Lv1NodeCapture {
            capture: HW_CONFIG_CAPTURE,
            address: HW_CONFIG_V1_ADDRESS,
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

/// The r3 of a read whose key names no node: `LV1_NO_ENTRY` (-6) with
/// 0x1E in the high word.
///
/// lv1's lookup returns it when the walk finds no node of the key
/// [OFW-4.93 lv1 a:0x2E3D38-0x2E3D40] [OFW-4.93 lv1 a:0x2E3E9C-0x2E3EB8].
/// The handler returns it with the values that it cleared before the
/// lookup, so r4 and r5 are 0 [OFW-4.93 lv1 a:0x2E52E0-0x2E5300]
/// [OFW-4.93 lv1 a:0x2E53A4-0x2E53B8].
pub const NO_NODE_R3: u64 = 0x0000_001E_FFFF_FFFA;

/// A key that a walk of the whole repository of a partition found no node
/// for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbsentNode {
    /// The key that names no node.
    pub key: RepositoryKey,
    /// The firmware versions whose read CellGov answers with
    /// [`NO_NODE_R3`], ascending.
    pub firmware: &'static [&'static str],
    /// The capture directory of the walk, relative to the workspace root.
    pub capture: &'static str,
}

/// A key of lpar 1's bus `bus` whose last three key words are `n`, which
/// the walk of [`BUS_CAPTURE`] found no node for.
const fn absent_bus_node(bus: u64, n: [u64; 3]) -> AbsentNode {
    AbsentNode {
        key: RepositoryKey {
            lpar: LPAR_ID_PME,
            n1: first_field(b"bus", bus),
            n2: n[0],
            n3: n[1],
            n4: n[2],
        },
        firmware: CAPTURE_FIRMWARE,
        capture: BUS_CAPTURE,
    }
}

/// Each key that a logged kernel boot reads and that a walk of the whole
/// repository found no node for, ascending by key.
///
/// The walk of [`BUS_CAPTURE`] went from the first dummy node to the
/// list's last node, and the capture keeps every node of buses 0 to 3.
/// A key of those buses that the capture does not hold therefore names no
/// node on the reference console.
pub const ABSENT_NODES: &[AbsentNode] = &[
    // `bus.1.dev.1.intr.2`: the read after the last interrupt node of
    // device 1 ends the handler's interrupt loop
    // [OFW-4.93 lv2_kernel a:0x80000000000935cc-0x80000000000935d4].
    absent_bus_node(1, [field(b"dev", 1), field(b"intr", 2), 0]),
    // `bus.1.dev.1.reg.2.type`: the read after the last region of device 1
    // ends the handler's region loop
    // [OFW-4.93 lv2_kernel a:0x8000000000093550-0x800000000009356c].
    absent_bus_node(1, [field(b"dev", 1), field(b"reg", 2), field(b"type", 0)]),
    // The same two reads for device 2.
    absent_bus_node(1, [field(b"dev", 2), field(b"intr", 2), 0]),
    absent_bus_node(1, [field(b"dev", 2), field(b"reg", 2), field(b"type", 0)]),
    // `bus.1.dev.3.reg.0.type`: the handler of type 6 reads it after the
    // id. After a failed read, the handler skips the region and reads interrupt 0
    // [OFW-4.93 lv2_kernel a:0x8000000000093364-0x8000000000093380].
    absent_bus_node(1, [field(b"dev", 3), field(b"reg", 0), field(b"type", 0)]),
];

/// The absent key `key` at the firmware version `firmware`, or `None` when
/// no walk shows that the key names no node at that firmware.
pub fn lookup_absent(key: &RepositoryKey, firmware: &str) -> Option<&'static AbsentNode> {
    ABSENT_NODES
        .iter()
        .find(|absent| absent.key == *key && absent.firmware.contains(&firmware))
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
#[path = "tests/repository_hw_config_tests.rs"]
mod hw_config_tests;

#[cfg(test)]
#[path = "tests/repository_update_status_tests.rs"]
mod update_status_tests;

#[cfg(test)]
#[path = "tests/repository_bus_tests.rs"]
mod bus_tests;

#[cfg(test)]
#[path = "tests/repository_storage_bus_tests.rs"]
mod storage_bus_tests;

#[cfg(test)]
#[path = "tests/repository_storage_bus_4_tests.rs"]
mod storage_bus_4_tests;

#[cfg(test)]
#[path = "tests/repository_bus_device_tests.rs"]
mod bus_device_tests;

#[cfg(test)]
#[path = "tests/repository_device_1_tests.rs"]
mod device_1_tests;
