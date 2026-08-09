//! End-to-end exercise of [`cellgov_ppu::prx_loader::load_firmware_set`]
//! against the user's full installed firmware corpus: every viable
//! `sys/external` module, selected by import-closure viability.
//!
//! The (namespace, NID) export key is what makes the full-install
//! load possible; a regression to NID-only resolution surfaces here
//! as `ConflictingExport` long before any title boot sees it.
//!
//! Requires the `firmware-corpus` feature and an installed firmware
//! set; `CELLGOV_FIRMWARE_DIR` overrides the default location.

#![allow(
    clippy::print_stderr,
    reason = "integration test: stderr carries the per-module load census"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use cellgov_mem::{GuestMemory, PageSize, Region};
use cellgov_ppu::prx_loader::{load_firmware_set, select_import_closure, PrxModuleId};

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if std::fs::read_to_string(p.join("Cargo.toml")).is_ok_and(|t| t.contains("[workspace]")) {
            return p;
        }
        if !p.pop() {
            panic!(
                "workspace root not found above {}",
                env!("CARGO_MANIFEST_DIR")
            );
        }
    }
}

fn locate_firmware_dir() -> PathBuf {
    let dir = match std::env::var("CELLGOV_FIRMWARE_DIR") {
        Ok(s) => PathBuf::from(s),
        Err(_) => workspace_root().join("firmware/sys/external"),
    };
    assert!(
        dir.is_dir(),
        "firmware dir not found: {}. Populate it with `cellgov_install install`,          or point CELLGOV_FIRMWARE_DIR at an existing install.",
        dir.display()
    );
    dir
}

#[test]
fn load_firmware_set_against_installed_corpus_is_coherent() {
    let dir = locate_firmware_dir();

    let mut candidates: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut sprx_paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read_dir on validated firmware dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("sprx"))
        })
        .collect();
    sprx_paths.sort();
    assert!(
        sprx_paths.len() >= 100,
        "expected a full retail install (100+ modules), found {} -- wrong directory?",
        sprx_paths.len()
    );
    for sprx_path in &sprx_paths {
        let raw = std::fs::read(sprx_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", sprx_path.display()));
        let elf = cellgov_install::sce::decrypt_self_to_elf(&raw)
            .unwrap_or_else(|e| panic!("decrypt {}: {e}", sprx_path.display()));
        let path_str = sprx_path
            .to_str()
            .unwrap_or_else(|| panic!("non-utf8 firmware path: {}", sprx_path.display()))
            .to_string();
        candidates.insert(path_str, elf);
    }

    let selection = select_import_closure(&candidates, None)
        .unwrap_or_else(|e| panic!("selection failed: {e}"));
    for (path, reason) in &selection.pruned {
        eprintln!("firmware_set_load: pruned {path}: {reason}");
    }
    assert_eq!(
        selection.selected.len() + selection.pruned.len(),
        candidates.len(),
        "selection accounting broken: every candidate is selected or pruned"
    );

    let mut bytes_by_path: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for path in &selection.selected {
        let elf = candidates
            .remove(path)
            .expect("selected path is a candidate");
        bytes_by_path.insert(path.clone(), elf);
    }
    eprintln!(
        "firmware_set_load: selected {} of {} modules ({} pruned)",
        bytes_by_path.len(),
        sprx_paths.len(),
        selection.pruned.len()
    );

    // Pre-parse the set of module ids the input is expected to
    // produce; the loader cannot independently witness it.
    let mut expected_ids: BTreeSet<PrxModuleId> = BTreeSet::new();
    for (path, bytes) in &bytes_by_path {
        let parsed =
            cellgov_ppu::sprx::parse_prx(bytes).unwrap_or_else(|e| panic!("parse {path}: {e:?}"));
        expected_ids.insert(parsed.module_id);
    }
    let n_inputs = bytes_by_path.len();

    let region_size: usize = 0x8000_0000;
    let mut memory =
        GuestMemory::from_regions(vec![Region::new(0, region_size, "main", PageSize::Page64K)])
            .expect("memory");
    let region_end: u64 = region_size as u64;
    let firmware_base: u64 = 0x4000_0000;

    let image = load_firmware_set(bytes_by_path, &mut memory, firmware_base)
        .unwrap_or_else(|e| panic!("load_firmware_set failed: {e:?}"));

    // (a) Identity bijection: loaded ids == input ids.
    let loaded_ids: BTreeSet<PrxModuleId> = image.loaded.keys().copied().collect();
    assert_eq!(image.loaded.len(), n_inputs, "loaded count != input count");
    assert_eq!(
        loaded_ids, expected_ids,
        "loaded ids differ from input ids (synthesized or dropped)"
    );

    // (b) Export-table == union of every module's (library, NID) pairs.
    let union: BTreeSet<(&str, u32)> = image
        .loaded
        .values()
        .flat_map(|p| {
            p.exports
                .iter()
                .flat_map(|(ns, by_nid)| by_nid.keys().map(move |&nid| (ns.as_str(), nid)))
        })
        .collect();
    let table_keys: BTreeSet<(&str, u32)> = image.export_table.keys().collect();
    assert_eq!(
        union, table_keys,
        "export table != union of per-module exports"
    );
    for prx in image.loaded.values() {
        for (namespace, by_nid) in &prx.exports {
            for (&nid, &opd) in by_nid {
                let table_opd = image
                    .export_table
                    .get(namespace, nid)
                    .expect("(namespace, nid) in table");
                assert_eq!(
                    table_opd, opd,
                    "table {namespace}::0x{nid:08x} maps to 0x{table_opd:x} but module {} carries 0x{opd:x}",
                    prx.name
                );
            }
        }
    }

    // (c) topological_order is a true permutation of loaded keys.
    let order_set: BTreeSet<PrxModuleId> = image.topological_order.iter().copied().collect();
    assert_eq!(
        order_set.len(),
        image.topological_order.len(),
        "topological_order has duplicates"
    );
    assert_eq!(
        order_set, loaded_ids,
        "topological_order keys != loaded keys"
    );

    // (d) Topological property: every import target precedes its
    // importer in the order.
    let position: BTreeMap<PrxModuleId, usize> = image
        .topological_order
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();
    for (importer, targets) in &image.imports_by_id {
        let importer_pos = position[importer];
        for target in targets {
            if let Some(&target_pos) = position.get(target) {
                assert!(
                    target_pos < importer_pos,
                    "topological order violated: target {target:?} at {target_pos} precedes importer {importer:?} at {importer_pos}"
                );
            }
        }
    }

    // (e) Layout: every module's [text_start, data_end) fits inside
    // the firmware region and ranges are pairwise disjoint.
    let mut ranges: Vec<(u64, u64, String)> = image
        .loaded
        .values()
        .map(|p| (p.text_start, p.data_end, p.name.clone()))
        .collect();
    for (start, end, name) in &ranges {
        assert!(
            *start >= firmware_base,
            "{name} starts at {start:#x} below firmware_base {firmware_base:#x}"
        );
        assert!(
            *end <= region_end,
            "{name} ends at {end:#x} past region end {region_end:#x}"
        );
        assert!(end > start, "{name} has empty/inverted range");
    }
    ranges.sort_by_key(|(s, _, _)| *s);
    for w in ranges.windows(2) {
        let (_, prev_end, prev_name) = &w[0];
        let (next_start, _, next_name) = &w[1];
        assert!(
            prev_end <= next_start,
            "modules overlap: {prev_name} ends at {prev_end:#x} but {next_name} starts at {next_start:#x}"
        );
    }

    eprintln!(
        "firmware_set_load: loaded {} PRX modules; export table {} (namespace, NID) pairs",
        image.loaded.len(),
        image.export_table.len()
    );
}

// `min_viable_prx_set_exports_required_namespaces` previously
// asserted a frozen `REQUIRED` namespace list (cellSysmodule,
// cellSysutil, cellGcmSys, cellSpurs, sys_io, cellSysutilAvconfExt)
// against the firmware corpus. That list was per-corpus
// compatibility state from the closure investigation, not a
// guard-liveness witness: a title-set churn that added or removed
// a stem from the closure walk would break this test without any
// loader / closure mechanism actually regressing. Since a
// correctness-improving trajectory shift must not break a
// liveness test, the assertion does not belong here.
//
// Deleted from this suite. The "every title in the corpus
// can resolve every namespace it imports" property belongs in the
// titles/compat suite where the inputs are titles + their imports
// rather than a hand-frozen list, so the assertion can derive its
// expectation from the same corpus state it validates. No
// relocation has been performed in this commit; this is a marker.
