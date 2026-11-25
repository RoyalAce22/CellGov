//! The address allocators of devices 0, 1 and 2 of bus 1 and lv1's I/O
//! segment table, held to the console capture of them after boot.

use std::collections::BTreeMap;

use super::*;

/// The global that holds the system bus object, from the TOC slot
/// r2 - 27872 [OFW-4.93 lv1 a:0x353358-0x35335C].
const SYSTEM_BUS_GLOBAL: u64 = 0x36_82C0;

/// The TOC slots r2 - 8480 and r2 - 8488, with lv1's TOC at 0x35A038
/// [OFW-4.93 lv1 a:0x351C6C]: the locations of the segment table's entry
/// count and of the table [OFW-4.93 lv1 a:0x2D9D20]
/// [OFW-4.93 lv1 a:0x2D9E1C].
const COUNT_TOC_SLOT: u64 = 0x35_7F18;
const TABLE_TOC_SLOT: u64 = 0x35_7F10;

/// The base of the backend's pool slots
/// [OFW-4.93 lv1 a:0x22D3B0-0x22D3F0].
const POOL_SLOTS: u64 = 32;

/// The offset in a pool of its allocator index k
/// [OFW-4.93 lv1 a:0x22CA84].
const POOL_KIND: u64 = 160;

/// The offsets in a pool of the two words that the address step
/// subtracts [OFW-4.93 lv1 a:0x22D584-0x22D598]
/// [OFW-4.93 lv1 a:0x22D6B8-0x22D6D4].
const POOL_BASE_WORDS: [u64; 2] = [168, 176];

/// The bus address that the address step adds
/// [OFW-4.93 lv1 a:0x22D6B8-0x22D6F0].
const BUS_OFFSET: u64 = 0x8000_0000;

/// The base of the backend's allocator table for the partition's calls
/// [OFW-4.93 lv1 a:0x22CCA8].
const PARTITION_ALLOCATORS: u64 = 328;

/// The base of the backend's allocator table for lv1's own calls. A
/// clear entry there gets an allocator on its first use
/// [OFW-4.93 lv1 a:0x22CAA4-0x22CAC4] [OFW-4.93 lv1 a:0x22CBD8-0x22CC04].
const LV1_ALLOCATORS: u64 = 192;

/// The method table of the 56-byte allocator, from the TOC slot
/// r2 - 27976 [OFW-4.93 lv1 a:0x22A1F0] [OFW-4.93 lv1 a:0x3532F0-0x3532F4].
const ALLOCATOR_CLASS: u64 = 0x35_C760;

/// The offsets in an allocator of its block list head
/// [OFW-4.93 lv1 a:0x22AD1C], and of its two ranges
/// [OFW-4.93 lv1 a:0x22A1FC-0x22A210].
const BLOCK_HEAD: u64 = 24;
const FIRST_RANGE: [u64; 2] = [8, 16];
const SECOND_RANGE: [u64; 2] = [32, 40];

/// The offsets in a block of the next block [OFW-4.93 lv1 a:0x22AD30],
/// its start and its size [OFW-4.93 lv1 a:0x22B128]
/// [OFW-4.93 lv1 a:0x22B150-0x22B154].
const BLOCK_NEXT: u64 = 32;
const BLOCK_START: u64 = 40;
const BLOCK_SIZE: u64 = 48;

/// The size of an I/O segment: the bound step shifts by 28
/// [OFW-4.93 lv1 a:0x2DA44C-0x2DA460].
const SEGMENT_SHIFT: u32 = 28;

/// The end that the segment pick caps at [OFW-4.93 lv1 a:0x2D9680-0x2D9684]
/// [OFW-4.93 lv1 a:0x2D9858].
const SEGMENT_END_CAP: u64 = 0x80;

/// The offset in the table's entry of its segment bitmap object
/// [OFW-4.93 lv1 a:0x2D953C] [OFW-4.93 lv1 a:0x2D968C].
const SEGMENT_BITMAP: u64 = 64;

/// The offsets in the bitmap object of its chunk array and chunk shift,
/// and in a chunk of its bit words [OFW-4.93 lv1 a:0x2C9DB4-0x2C9DF0]
/// [OFW-4.93 lv1 a:0x2C9E14].
const CHUNKS: u64 = 24;
const CHUNK_SHIFT: u64 = 32;
const CHUNK_BITS: u64 = 64;

/// One device of bus 1: its pool index, its allocator index k, and its
/// two ranges.
struct Device {
    device: u64,
    pool_index: u64,
    kind: u64,
    first: (u64, u64),
    second: (u64, u64),
}

/// The pool index of each device [OFW-4.93 lv1 a:0x22B838-0x22B844]
/// [OFW-4.93 lv1 a:0x3A9AC4], the word k that lv1's init gives it
/// [OFW-4.93 lv1 a:0x3A9924-0x3A9938] [OFW-4.93 lv1 a:0x3A9A4C-0x3A9A5C]
/// [OFW-4.93 lv1 a:0x3A9AA0-0x3A9AB0], and the ranges of k
/// [OFW-4.93 lv1 a:0x22CD38-0x22CD54] [OFW-4.93 lv1 a:0x22CD80]
/// [OFW-4.93 lv1 a:0x22CD9C-0x22CDAC] [OFW-4.93 lv1 a:0x22CDE4-0x22CDF8]
/// [OFW-4.93 lv1 a:0x22CDFC-0x22CE10].
const DEVICES: [Device; 3] = [
    Device {
        device: 0,
        pool_index: 0,
        kind: 2,
        first: (0x1_4000_0000, 0x4000_0000),
        second: (0x1_2000_0000, 0x2000_0000),
    },
    Device {
        device: 1,
        pool_index: 3,
        kind: 8,
        first: (0x4_4000_0000, 0x4000_0000),
        second: (0x4_2000_0000, 0x2000_0000),
    },
    Device {
        device: 2,
        pool_index: 4,
        kind: 9,
        first: (0x5_4000_0000, 0x4000_0000),
        second: (0x5_2000_0000, 0x2000_0000),
    },
];

/// The words of every file of the capture, by address.
///
/// Both reads of each word must agree, and no address is in two files.
fn words() -> BTreeMap<u64, u64> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(DMA_SEGMENTS_CAPTURE)
        .join("peek/cech20-cex-493");
    let mut words = BTreeMap::new();
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut files = 0;
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        let name = path.display().to_string();
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut lines = text.lines();
        let header = lines.next();
        assert!(
            header == Some("address\treads\tagree\tdoubleword")
                || header == Some("address\treads\tagree\tword"),
            "{name}: header {header:?}"
        );
        let hex = |cell: &str| {
            let digits = cell
                .strip_prefix("0x")
                .unwrap_or_else(|| panic!("{name}: {cell} is not 0x hex"));
            u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{name}: {cell}: {e}"))
        };
        for line in lines {
            let row: Vec<&str> = line.split('\t').collect();
            assert_eq!(row.len(), 4, "{name}: {line}");
            assert_eq!(row[1..3], ["2", "yes"], "{name}: {line}");
            assert_eq!(
                words.insert(hex(row[0]), hex(row[3])),
                None,
                "{name}: {line}"
            );
        }
        files += 1;
    }
    assert_eq!(files, 13, "{}", dir.display());
    words
}

/// The captured word at `address`.
fn word(words: &BTreeMap<u64, u64>, address: u64) -> u64 {
    *words
        .get(&address)
        .unwrap_or_else(|| panic!("no captured word at {address:#x}"))
}

/// The pool of `device`, from the system bus object.
fn pool(words: &BTreeMap<u64, u64>, device: &Device) -> u64 {
    let bus = word(words, SYSTEM_BUS_GLOBAL);
    word(words, bus + POOL_SLOTS + 8 * device.pool_index)
}

/// The partition allocator of `device`, after a check of its class.
fn allocator(words: &BTreeMap<u64, u64>, device: &Device) -> u64 {
    let bus = word(words, SYSTEM_BUS_GLOBAL);
    let allocator = word(words, bus + PARTITION_ALLOCATORS + 8 * device.kind);
    assert_eq!(
        word(words, allocator),
        ALLOCATOR_CLASS,
        "device {}",
        device.device
    );
    allocator
}

/// The start and size of each block of `device`'s allocator, in list
/// order.
fn blocks(words: &BTreeMap<u64, u64>, device: &Device) -> Vec<(u64, u64)> {
    let mut blocks = Vec::new();
    let mut block = word(words, allocator(words, device) + BLOCK_HEAD);
    for _ in 0..16 {
        if block == 0 {
            break;
        }
        blocks.push((
            word(words, block + BLOCK_START),
            word(words, block + BLOCK_SIZE),
        ));
        block = word(words, block + BLOCK_NEXT);
    }
    assert_eq!(block, 0, "device {}: the list ends", device.device);
    blocks
}

/// Whether the captured segment bitmap holds segment `segment` as taken.
///
/// A set bit marks a free segment: the claim succeeds only on a set bit
/// and clears it [OFW-4.93 lv1 a:0x2CA160-0x2CA198].
fn segment_held(words: &BTreeMap<u64, u64>, segment: u64) -> bool {
    let table = word(words, word(words, TABLE_TOC_SLOT));
    let bitmap = word(words, table) + SEGMENT_BITMAP;
    let shift = word(words, bitmap + CHUNK_SHIFT);
    let (chunk, base) = if segment < 1 << shift {
        (0, 0)
    } else {
        let chunk = u64::from(64 - segment.leading_zeros()) - shift;
        (chunk, 1 << (chunk + shift - 1))
    };
    let chunk = word(words, word(words, bitmap + CHUNKS) + 8 * chunk);
    let bits = word(words, chunk + CHUNK_BITS);
    let offset = segment - base;
    word(words, bits + 8 * (offset >> 6)) & (1 << (offset & 63)) == 0
}

/// The segments `[first, end)` that a range covers.
fn segments((base, size): (u64, u64)) -> std::ops::Range<u64> {
    (base >> SEGMENT_SHIFT)..((base + size) >> SEGMENT_SHIFT)
}

/// The segments that hold a byte of the block at `start` of `length`
/// bytes. A block can be smaller than one segment.
fn block_segments((start, length): (u64, u64)) -> std::ops::RangeInclusive<u64> {
    (start >> SEGMENT_SHIFT)..=((start + length - 1) >> SEGMENT_SHIFT)
}

#[test]
fn the_captured_segment_table_has_one_entry() {
    let words = words();
    assert_eq!(word(&words, word(&words, COUNT_TOC_SLOT)), 1);
}

#[test]
fn each_device_has_a_captured_partition_allocator_with_the_ranges_of_its_k() {
    let words = words();
    for device in &DEVICES {
        let pool = pool(&words, device);
        assert_eq!(
            word(&words, pool + POOL_KIND),
            device.kind,
            "device {}",
            device.device
        );
        let allocator = allocator(&words, device);
        let range = |[base, size]: [u64; 2]| {
            (
                word(&words, allocator + base),
                word(&words, allocator + size),
            )
        };
        assert_eq!(range(FIRST_RANGE), device.first, "device {}", device.device);
        assert_eq!(
            range(SECOND_RANGE),
            device.second,
            "device {}",
            device.device
        );
    }
}

#[test]
fn the_captured_segments_of_devices_0_1_and_2_do_not_overlap_and_stay_below_the_cap() {
    let words = words();
    let spans: Vec<std::ops::Range<u64>> = DEVICES
        .iter()
        .map(|device| {
            let allocator = allocator(&words, device);
            let range = |[base, size]: [u64; 2]| {
                segments((
                    word(&words, allocator + base),
                    word(&words, allocator + size),
                ))
            };
            let (first, second) = (range(FIRST_RANGE), range(SECOND_RANGE));
            assert!(
                !first.is_empty() && !second.is_empty(),
                "device {}",
                device.device
            );
            second.start.min(first.start)..second.end.max(first.end)
        })
        .collect();
    assert_eq!(spans.len(), 3);
    for (i, a) in spans.iter().enumerate() {
        assert!(a.end <= SEGMENT_END_CAP, "{a:?}");
        for b in &spans[i + 1..] {
            assert!(a.end <= b.start || b.end <= a.start, "{a:?} and {b:?}");
        }
    }
}

#[test]
fn each_captured_block_lies_in_its_devices_first_range_and_holds_its_segments() {
    let words = words();
    let counts: Vec<usize> = DEVICES
        .iter()
        .map(|device| blocks(&words, device).len())
        .collect();
    assert_eq!(counts, [1, 3, 3]);
    let expected: [&[u64]; 3] = [&[0x14], &[0x44, 0x45, 0x46], &[0x54, 0x55, 0x56]];
    for (device, expected) in DEVICES.iter().zip(expected) {
        let (base, size) = device.first;
        let mut held = Vec::new();
        for (start, length) in blocks(&words, device) {
            assert!(
                base <= start && start + length <= base + size,
                "device {}: {start:#x}",
                device.device
            );
            for segment in block_segments((start, length)) {
                assert!(
                    segment_held(&words, segment),
                    "device {}: {segment:#x}",
                    device.device
                );
                held.push(segment);
            }
        }
        assert_eq!(held, expected, "device {}", device.device);
    }
}

#[test]
fn devices_1_and_2_have_no_captured_allocator_in_lv1s_own_table() {
    let words = words();
    let bus = word(&words, SYSTEM_BUS_GLOBAL);
    for device in &DEVICES[1..] {
        assert_eq!(
            word(&words, bus + LV1_ALLOCATORS + 8 * device.kind),
            0,
            "device {}",
            device.device
        );
    }
}

#[test]
fn each_captured_region_address_maps_into_a_block_of_its_own_device() {
    let words = words();
    for device in &DEVICES {
        let pool = pool(&words, device);
        let base = POOL_BASE_WORDS
            .iter()
            .map(|offset| word(&words, pool + offset))
            .sum::<u64>();
        let blocks = blocks(&words, device);
        let rows: Vec<GameOsDmaRegion> = game_os_dma_regions("4.93", 1, device.device).collect();
        assert!(!rows.is_empty(), "device {}", device.device);
        for row in rows {
            let found = row.address - BUS_OFFSET + base;
            assert!(
                blocks
                    .iter()
                    .any(|&(start, length)| start <= found && found + row.size <= start + length),
                "device {}: {:#x}",
                device.device,
                row.address
            );
        }
    }
}
