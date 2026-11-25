//! The repository nodes served from a console capture, against the
//! committed capture itself.
//!
//! The test reads only committed files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;
use cellgov_ps3_abi::lv1::repository::{
    field, first_field, lookup, NodeSource, RepositoryKey, BOOT_RECORD_4_93, BOOT_RECORD_CAPTURE,
    NODES,
};

fn workspace_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// The capture's reads, as address -> doubleword.
fn captured_words(capture: &str) -> BTreeMap<u64, u64> {
    let path = workspace_file(capture).join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    assert_eq!(frame.len(), 8 + len, "one whole frame");
    assert_eq!(len % 16, 0, "whole (address, doubleword) pairs");
    let word = |at: usize| u64::from_be_bytes(frame[at..at + 8].try_into().expect("8 bytes"));
    let words: BTreeMap<u64, u64> = (8..frame.len())
        .step_by(16)
        .map(|at| (word(at), word(at + 8)))
        .collect();
    assert_eq!(
        words.len(),
        len / 16,
        "{} reads one address twice",
        path.display()
    );
    words
}

fn captured_firmware(capture: &str) -> String {
    let path = workspace_file(capture).join("provenance.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let provenance: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    provenance["console"]["firmware"]
        .as_str()
        .unwrap_or_else(|| panic!("{} names no console firmware", path.display()))
        .to_string()
}

#[test]
fn every_boot_record_row_is_the_doubleword_its_capture_read_at_the_capture_firmware() {
    let rows: Vec<_> = NODES
        .iter()
        .filter_map(|node| match node.source {
            NodeSource::ConsoleCapture { capture, address } => Some((node, capture, address)),
            NodeSource::PsdevwikiDump { .. } | NodeSource::Lv1NodeCapture { .. } => None,
        })
        .collect();
    assert_eq!(rows.len(), 7);
    for (node, capture, address) in rows {
        let words = captured_words(capture);
        assert_eq!(
            words.get(&address),
            Some(&node.v1),
            "{} at {address:#018x} in {capture}",
            node.key
        );
        assert_eq!(node.firmware, [captured_firmware(capture).as_str()]);
    }
}

#[test]
fn the_4_93_boot_record_capture_agrees_with_the_record_the_kernel_derives() {
    let words = captured_words(BOOT_RECORD_CAPTURE);
    assert_eq!(words.len(), 27);
    let at = |offset: u64| {
        *words
            .get(&(BOOT_RECORD_4_93 + offset))
            .unwrap_or_else(|| panic!("the capture holds no record word {offset:#x}"))
    };
    // The TOC slot and the kernel's pointer both locate the record.
    assert_eq!(words.get(&0x8000_0000_0034_7f70), Some(&BOOT_RECORD_4_93));
    assert_eq!(words.get(&0x8000_0000_0037_a590), Some(&BOOT_RECORD_4_93));

    let (rgntotal, rm_size) = (at(0x68), at(0x38));
    let remainder = rgntotal & 0xf_ffff;
    assert_eq!(at(0x08), LPAR_ID_GAME_OS);
    assert_eq!(at(0x80), rm_size);
    assert_eq!(at(0xa0), remainder);
    assert_eq!(at(0x90), rgntotal - rm_size - remainder);
    assert_eq!(at(0x70), if remainder == 0 { 2 } else { 3 });

    // Three of the record's repository values are the dump values that
    // CellGov serves to the older firmware.
    let bi = |second: &[u8], n3: u64, n4: &[u8]| RepositoryKey {
        lpar: LPAR_ID_GAME_OS,
        n1: first_field(b"bi", 0),
        n2: field(second, 0),
        n3,
        n4: field(n4, 0),
    };
    let dump = |key: RepositoryKey| lookup(&key, "3.70").map(|node| node.v1);
    assert_eq!(dump(bi(b"pun", 0, b"")), Some(at(0x10)));
    assert_eq!(dump(bi(b"pu", 0, b"")), Some(at(0x18)));
    assert_eq!(dump(bi(b"pu", 1, b"rm_addr")), Some(at(0x30)));
}
