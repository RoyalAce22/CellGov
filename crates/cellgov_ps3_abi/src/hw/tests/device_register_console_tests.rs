//! The register words of device 1 of bus 1, held to the console capture
//! of the kernel's copies of them.

use std::collections::BTreeMap;

use super::*;
use crate::lv1::device_bus::{mmio_region, MMIO_REGIONS};

/// The doubleword that the kernel loads into r2
/// [OFW-4.93 lv2_kernel a:0x8000000000003000].
const TOC_WORD: u64 = 0x8000_0000_0000_3000;

/// The offset of the TOC slot that holds the address of the global
/// pointer to the controller manager
/// [OFW-4.93 lv2_kernel a:0x800000000024FF98].
const MANAGER_SLOT: u64 = 2944;

/// The class that the controller constructor stores at +0, from the TOC
/// slot r2 + 2664 [OFW-4.93 lv2_kernel a:0x8000000000243D54].
const CONTROLLER_CLASS: u64 = 0x8000_0000_0036_3760;

/// The doublewords of every file of the capture, by address.
///
/// The column `address` holds each address, and the last column holds
/// the doubleword. Both reads of each doubleword must agree.
fn words() -> BTreeMap<u64, u64> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(DEVICE_REGISTER_CAPTURE)
        .join("peek/cech20-cex-493");
    let files = [
        ("globals.tsv", 3),
        ("manager.tsv", 6),
        ("controllers.tsv", 12),
        ("devices.tsv", 2),
        ("records.tsv", 6),
    ];
    let mut words = BTreeMap::new();
    for (file, rows) in files {
        let path = dir.join(file);
        let name = path.display().to_string();
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut lines = text.lines();
        let header: Vec<&str> = lines.next().expect("a header").split('\t').collect();
        let column = |label: &str| {
            header
                .iter()
                .position(|cell| *cell == label)
                .unwrap_or_else(|| panic!("{name}: no {label} column"))
        };
        let (address, reads, agree) = (column("address"), column("reads"), column("agree"));
        let hex = |text: &str| {
            u64::from_str_radix(text.trim_start_matches("0x"), 16)
                .unwrap_or_else(|e| panic!("{name}: {text}: {e}"))
        };
        let mut count = 0;
        for line in lines {
            let row: Vec<&str> = line.split('\t').collect();
            assert_eq!(row.len(), header.len(), "{name}: {line}");
            assert_eq!((row[reads], row[agree]), ("2", "yes"), "{name}: {line}");
            let value = hex(row[row.len() - 1]);
            assert_eq!(
                words.insert(hex(row[address]), value),
                None,
                "{name}: {line}"
            );
            count += 1;
        }
        assert_eq!(count, rows, "{name}");
    }
    words
}

/// What the capture gives for one controller: the bus address of its
/// region, the low byte of word +0 and the low 4 bits of word +4.
#[derive(Debug, PartialEq, Eq)]
struct Copies {
    region: u64,
    word_0: u32,
    word_4: u32,
}

/// The copies of the controller at `controller`, through its device's
/// mapping record.
fn copies(words: &BTreeMap<u64, u64>, controller: u64) -> Copies {
    let at = |addr: u64| words[&addr];
    assert_eq!(at(controller), CONTROLLER_CLASS);
    let record = at(at(controller + 40) + 80);
    let (region, ea, len) = (at(record + 48), at(record + 56), at(record + 64));
    let held = MMIO_REGIONS
        .iter()
        .any(|r| r.firmware == "4.93" && r.bus == 1 && r.address == region && r.size == len);
    assert!(held, "region 0x{region:x} of 0x{len:x} bytes");
    let offset = at(controller + 128) - ea;
    assert!(offset < 0x100, "offset 0x{offset:x}");
    Copies {
        region,
        word_0: offset as u32,
        word_4: (at(controller + 48) >> 56) as u32,
    }
}

#[test]
fn both_controllers_hold_the_offset_0x10_and_the_count_2() {
    let words = words();
    let toc = words[&TOC_WORD];
    let manager = words[&words[&(toc + MANAGER_SLOT)]];
    let first = copies(&words, words[&(manager + 8)]);
    let second = copies(&words, words[&(manager + 24)]);
    let expect = |region| Copies {
        region,
        word_0: 0x10,
        word_4: 2,
    };
    assert_eq!(first, expect(0x381_0000));
    assert_eq!(second, expect(0x382_0000));
}

#[test]
fn each_register_read_is_the_captured_bits_of_device_1() {
    let words = words();
    let toc = words[&TOC_WORD];
    let manager = words[&words[&(toc + MANAGER_SLOT)]];
    let device_1 = copies(&words, words[&(manager + 8)]);
    assert_eq!(DEVICE_REGISTER_READS.len(), 2);
    for read in DEVICE_REGISTER_READS {
        assert_eq!((read.firmware, read.bus, read.device), ("4.93", 1, 1));
        assert_eq!(read.region, device_1.region);
        // The copy keeps the low byte of word +0
        // [OFW-4.93 lv2_kernel a:0x8000000000245914] clrldi r0,r0,56, and
        // the low 4 bits of word +4
        // [OFW-4.93 lv2_kernel a:0x8000000000245918] clrlwi r9,r9,28.
        let (captured, kept) = match read.offset {
            0 => (device_1.word_0, 0xFF),
            4 => (device_1.word_4, 0xF),
            other => panic!("no capture of the word at +0x{other:x}"),
        };
        assert_eq!(read.known, kept, "{read:?}");
        assert_eq!(read.value, captured & read.known, "{read:?}");
        assert_eq!(read.value & !read.known, 0, "{read:?}");
    }
}

#[test]
fn each_register_read_is_an_aligned_word_of_its_region() {
    assert_eq!(DEVICE_REGISTER_READS.len(), 2);
    for read in DEVICE_REGISTER_READS {
        let region = mmio_region(read.firmware, read.bus, read.device, read.region)
            .unwrap_or_else(|| panic!("no region for {read:?}"));
        assert!(read.offset.is_multiple_of(4), "{read:?}");
        assert!(read.offset + 4 <= region.size, "{read:?}");
        assert!(read.reader.is_multiple_of(4), "{read:?}");
    }
}
