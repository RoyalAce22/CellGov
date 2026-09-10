//! `NID_TABLE` reconciled against the SHA-1 derivation and the curated consts.

use super::*;
use crate::format::elf::{NID_MODULE_START, NID_MODULE_STOP};
use crate::sha1::nid_sha1;

/// The module entry points: the only rows whose key is a fixed value
/// rather than `nid_sha1(name)`.
const ENTRY_POINT_NIDS: &[(u32, &str)] = &[
    (NID_MODULE_START, "module_start"),
    (NID_MODULE_STOP, "module_stop"),
    (0x3ab9_a95e, "module_exit"),
    (0xd7f4_3016, "module_info"),
    (0x0d10_fd3f, "module_prologue"),
    (0x330f_7005, "module_epilogue"),
];

/// A row whose real name is unknown carries `<module>_<NID:08X>` as
/// its name; the suffix restates the key.
fn is_placeholder_name(nid: u32, module: &str, name: &str) -> bool {
    !module.is_empty() && name == format!("{module}_{nid:08X}")
}

#[test]
fn every_nid_module_block_is_listed_in_curated() {
    let blocks = include_str!("../modules.rs")
        .matches("crate::nid_module! {")
        .count();
    assert_eq!(
        blocks,
        CURATED.len(),
        "modules.rs declares {blocks} nid_module! blocks but CURATED lists {}; \
         a block missing from CURATED is never reconciled against NID_TABLE",
        CURATED.len(),
    );
}

#[test]
fn table_is_sorted_with_unique_keys() {
    for pair in NID_TABLE.windows(2) {
        assert!(
            pair[0].0 < pair[1].0,
            "NID_TABLE is not strictly ascending at 0x{:08x} ({:?}) -> 0x{:08x} ({:?}); \
             `lookup` binary-searches it",
            pair[0].0,
            pair[0].2,
            pair[1].0,
            pair[1].2,
        );
    }
}

#[test]
fn every_table_row_recomputes_to_its_key() {
    let mut unmatched = Vec::new();
    let mut derived = 0usize;
    let mut placeholders = 0usize;
    let mut entry_points = 0usize;
    for &(nid, module, name) in NID_TABLE {
        if name.is_empty() {
            unmatched.push(format!(
                "0x{nid:08x} {module:?}: empty name, nothing to recompute"
            ));
        } else if nid_sha1(name) == nid {
            derived += 1;
        } else if is_placeholder_name(nid, module, name) {
            placeholders += 1;
        } else if ENTRY_POINT_NIDS.contains(&(nid, name)) {
            entry_points += 1;
        } else {
            unmatched.push(format!(
                "0x{nid:08x} {module:?} {name:?}: SHA-1(name || salt) is 0x{:08x}, and the \
                 row is neither a placeholder name nor a listed entry point",
                nid_sha1(name),
            ));
        }
    }
    assert!(
        unmatched.is_empty(),
        "{} of {} NID_TABLE rows do not recompute to their key:\n{}",
        unmatched.len(),
        NID_TABLE.len(),
        unmatched.join("\n"),
    );
    assert_eq!(
        derived + placeholders + entry_points,
        NID_TABLE.len(),
        "every row is classified exactly once"
    );
    assert_eq!(
        entry_points,
        ENTRY_POINT_NIDS.len(),
        "every listed entry point has its NID_TABLE row"
    );
}

#[test]
fn entry_point_nids_are_not_named_export_hashes() {
    for &(nid, name) in ENTRY_POINT_NIDS {
        assert_ne!(
            nid_sha1(name),
            nid,
            "{name} derives from the named-export salt; drop it from ENTRY_POINT_NIDS"
        );
        assert_eq!(lookup(nid), Some(("", name)));
    }
}

#[test]
fn every_declared_const_matches_its_table_row() {
    let mut disagreements = Vec::new();
    let mut checked = 0usize;
    for (module, declared) in CURATED {
        for &(nid, name) in *declared {
            checked += 1;
            match lookup(nid) {
                Some((_, row_name)) if row_name == name => {}
                Some((_, row_name)) => disagreements.push(format!(
                    "{module}: 0x{nid:08x} declared as {name:?}, NID_TABLE row says {row_name:?}"
                )),
                None => disagreements.push(format!(
                    "{module}: 0x{nid:08x} {name:?} has no NID_TABLE row"
                )),
            }
        }
    }
    assert!(checked > 0, "no nid_module! block is listed in CURATED");
    assert!(
        disagreements.is_empty(),
        "{} of {checked} declared NIDs disagree with NID_TABLE:\n{}",
        disagreements.len(),
        disagreements.join("\n"),
    );
}
