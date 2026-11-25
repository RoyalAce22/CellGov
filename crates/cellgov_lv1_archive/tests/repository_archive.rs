//! The rules of the repository tables that span rows and tables: the
//! logged boots, the text form of each key, the order of each boot's
//! reads and the source of each served node.
//!
//! The test reads only committed files.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_lv1_archive::{
    self as archive, REPOSITORY_GATE, REPOSITORY_LOGGED_FIRMWARE, REPOSITORY_READ,
    REPOSITORY_SOURCES,
};
use cellgov_lv2_archive::{self as lv2_archive, KERNEL, PUP};
use cellgov_ps3_abi::lv1::repository::{NodeSource, ABSENT_NODES, NODES};

fn archive_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lv1")
}

/// The LV2 archive, which holds the PUP and kernel tables.
fn lv2_archive_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lv2")
}

fn read_in(dir: &Path, file: &str) -> String {
    let path = dir.join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn repository_rows_are_well_formed() {
    assert_eq!(REPOSITORY_READ.gate, REPOSITORY_GATE);
    let reads = archive::repository_read_rows(
        &lv2_archive::parse(
            &REPOSITORY_READ,
            &read_in(&archive_dir(), &REPOSITORY_READ.file()),
        )
        .unwrap_or_else(|e| panic!("{e}")),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let pups = lv2_archive::pup_rows(
        &lv2_archive::parse(&PUP, &read_in(&lv2_archive_dir(), &PUP.file()))
            .unwrap_or_else(|e| panic!("{e}")),
    );
    let kernel_pups: BTreeSet<String> = lv2_archive::kernel_rows(
        &lv2_archive::parse(&KERNEL, &read_in(&lv2_archive_dir(), &KERNEL.file()))
            .unwrap_or_else(|e| panic!("{e}")),
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .into_iter()
    .map(|row| row.pup_sha256)
    .collect();
    let fw_of: BTreeMap<&str, &str> = pups
        .iter()
        .map(|row| (row.pup_sha256.as_str(), row.fw.as_str()))
        .collect();

    // One logged boot of one kernel.tsv kernel for each logged firmware.
    let mut boots: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for row in &reads {
        let fw = fw_of
            .get(row.pup_sha256.as_str())
            .unwrap_or_else(|| panic!("read row names a PUP pup.tsv lacks: {row:?}"));
        assert!(
            kernel_pups.contains(&row.pup_sha256),
            "read row names a PUP with no kernel.tsv kernel: {row:?}"
        );
        boots.entry(fw).or_default().insert(&row.pup_sha256);
    }
    assert_eq!(
        boots.keys().copied().collect::<Vec<_>>(),
        REPOSITORY_LOGGED_FIRMWARE,
        "repository_read.tsv logs other firmware than the logged set"
    );
    for (fw, pups) in &boots {
        assert_eq!(
            pups.len(),
            1,
            "firmware {fw} has {} logged boots",
            pups.len()
        );
    }

    // The text keeps the index, and the raw words stay the key.
    for row in &reads {
        assert_eq!(row.node, row.key.path(), "{row:?}");
    }

    // Each boot's first reads are distinct, and a refused read is the
    // boot's last: the refusal stops the run.
    for pup in boots.values().flatten() {
        let boot: Vec<_> = reads.iter().filter(|r| r.pup_sha256 == *pup).collect();
        assert!(!boot.is_empty());
        let firsts: BTreeSet<usize> = boot.iter().map(|r| r.first).collect();
        assert_eq!(firsts.len(), boot.len(), "PUP {pup} repeats a first read");
        let last = *firsts.last().expect("the boot read a node");
        for row in boot.iter().filter(|r| !r.answered) {
            assert_eq!(row.first, last, "a refused read is not the last: {row:?}");
        }
    }

    // Every served node names a source of a known class.
    assert!(!NODES.is_empty());
    for node in NODES {
        assert!(
            REPOSITORY_SOURCES.contains(&node.source.class()),
            "{} names the source class {}",
            node.key,
            node.source.class()
        );
        match node.source {
            NodeSource::PsdevwikiDump { alias, section } => {
                let page = alias.strip_prefix("PS3DEVWIKI-");
                assert!(
                    page.is_some_and(|p| !p.is_empty()
                        && p.bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')),
                    "{} cites {alias}, which is no psdevwiki citation alias",
                    node.key
                );
                assert!(!section.is_empty(), "{} names no section", node.key);
            }
            NodeSource::ConsoleCapture { capture, .. } => {
                assert!(
                    archive_dir()
                        .join("../..")
                        .join(capture)
                        .join("cgov_frame.bin")
                        .is_file(),
                    "{} cites {capture}, which holds no committed frame",
                    node.key
                );
            }
            NodeSource::Lv1NodeCapture { capture, .. } => {
                assert!(
                    archive_dir()
                        .join("../..")
                        .join(capture)
                        .join("node.tsv")
                        .is_file(),
                    "{} cites {capture}, which holds no committed node.tsv",
                    node.key
                );
            }
        }
    }

    // The node table and the absent-key table answer exactly the answered
    // reads.
    archive::repository_rows(NODES, ABSENT_NODES, &reads, &pups).unwrap_or_else(|e| panic!("{e}"));
}
