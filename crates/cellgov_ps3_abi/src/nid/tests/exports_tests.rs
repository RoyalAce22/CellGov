//! `NID_TABLE`'s module column joined against the 4.93 firmware export
//! tables in `exports_493.tsv`.

use std::collections::{BTreeMap, BTreeSet};

use super::*;

const EXPORTS: &str = include_str!("../exports_493.tsv");

/// Table rows whose module a 4.93 library exports the NID under.
const CONFIRMED_ROWS: usize = 3522;

/// Table rows that name a module no 4.93 module exports the NID from.
const UNEXPORTED_ROWS: usize = 113;

/// The libraries that export each NID, the nameless library as `""`.
fn exporters() -> BTreeMap<u32, BTreeSet<&'static str>> {
    let mut lines = EXPORTS.lines();
    assert_eq!(
        lines.next(),
        Some("module\tlibrary\tnid\tkind"),
        "exports_493.tsv starts with its column header"
    );
    let mut by_nid: BTreeMap<u32, BTreeSet<&'static str>> = BTreeMap::new();
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        let [_module, library, nid, kind] = fields[..] else {
            panic!("exports_493.tsv: expected 4 tab-separated fields, got {line:?}");
        };
        assert!(
            kind == "func" || kind == "var",
            "exports_493.tsv: kind {kind:?} in {line:?}"
        );
        let nid = u32::from_str_radix(nid, 16)
            .unwrap_or_else(|e| panic!("exports_493.tsv: nid {nid:?}: {e}"));
        by_nid.entry(nid).or_default().insert(library);
    }
    by_nid
}

fn is_placeholder(nid: u32, module: &str, name: &str) -> bool {
    !module.is_empty() && name == format!("{module}_{nid:08X}")
}

#[test]
fn every_module_column_names_a_library_that_exports_the_nid() {
    let exporters = exporters();
    let mut wrong = Vec::new();
    let (mut confirmed, mut unexported) = (0usize, 0usize);
    for &(nid, module, name) in NID_TABLE {
        if module.is_empty() {
            continue;
        }
        match exporters.get(&nid) {
            Some(libraries) if libraries.contains(module) => confirmed += 1,
            Some(libraries) => wrong.push(format!(
                "0x{nid:08x} {name:?}: the row says {module:?}, 4.93 exports it from {libraries:?}"
            )),
            None => unexported += 1,
        }
    }
    assert!(
        wrong.is_empty(),
        "{} NID_TABLE rows name a module 4.93 does not export them from:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    assert_eq!(
        (confirmed, unexported),
        (CONFIRMED_ROWS, UNEXPORTED_ROWS),
        "the confirmed and unexported counts moved; update them here and in nid/mod.rs"
    );
}

#[test]
fn no_row_leaves_out_the_library_that_exports_it() {
    let exporters = exporters();
    assert_eq!(
        NID_TABLE.iter().filter(|r| r.1.is_empty()).count(),
        1692,
        "rows with no module; update nid/mod.rs"
    );
    let missing: Vec<String> = NID_TABLE
        .iter()
        .filter(|(_, module, _)| module.is_empty())
        .filter_map(|&(nid, _, name)| {
            let named: Vec<&str> = exporters
                .get(&nid)?
                .iter()
                .copied()
                .filter(|library| !library.is_empty())
                .collect();
            (!named.is_empty()).then(|| format!("0x{nid:08x} {name:?} is exported from {named:?}"))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "{} NID_TABLE rows have an empty module although a named 4.93 library exports them:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn every_placeholder_is_exported_by_its_module() {
    let exporters = exporters();
    let mut placeholders = 0usize;
    for &(nid, module, name) in NID_TABLE {
        if !is_placeholder(nid, module, name) {
            continue;
        }
        placeholders += 1;
        assert!(
            exporters
                .get(&nid)
                .is_some_and(|libraries| libraries.contains(module)),
            "placeholder {name:?} is not exported by {module:?} on 4.93"
        );
    }
    assert_eq!(placeholders, 18, "the placeholder rows checked");
}
