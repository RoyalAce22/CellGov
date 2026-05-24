//! End-to-end exercise of [`cellgov_ppu::prx_loader::load_firmware_set`]
//! against the user's installed firmware corpus, scoped to the
//! minimum viable PRX set.
//!
//! Skipped silently when the firmware directory or any required PRX
//! stem is absent. `CELLGOV_REQUIRE_FIRMWARE_SET_LOAD=1` promotes
//! both conditions to a hard failure (CI knob).

#![allow(
    clippy::print_stderr,
    reason = "integration test: stderr carries fixture-absent diagnostics"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use cellgov_mem::{GuestMemory, PageSize, Region};
use cellgov_ppu::prx_loader::{check_loadable, load_firmware_set, PrxLoaderError, PrxModuleId};

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

use cellgov_ppu::prx_loader::MIN_VIABLE_PRX_STEMS;

/// Expected count of minimum-viable PRXes filtered by
/// `MultiSegmentRelocations` on the current firmware install. Drift
/// in either direction is the test signal: a regression that
/// suddenly rejects more modules trips, and a hand-rolled exception
/// that smuggles a multi-segment module through silently also trips.
const MULTI_SEG_EXPECTED: usize = 0;

fn locate_firmware_dir() -> Option<PathBuf> {
    let dir = match std::env::var("CELLGOV_FIRMWARE_DIR") {
        Ok(s) => PathBuf::from(s),
        Err(_) => workspace_root().join("firmware/sys/external"),
    };
    if dir.is_dir() {
        Some(dir)
    } else {
        if std::env::var_os("CELLGOV_REQUIRE_FIRMWARE_SET_LOAD").is_some() {
            panic!(
                "CELLGOV_REQUIRE_FIRMWARE_SET_LOAD set but firmware dir not found: {}",
                dir.display()
            );
        }
        eprintln!(
            "firmware_set_load: skipping (firmware dir {} absent; \
             run `cellgov_firmware install` to populate)",
            dir.display()
        );
        None
    }
}

#[test]
fn load_firmware_set_against_installed_corpus_is_coherent() {
    let Some(dir) = locate_firmware_dir() else {
        return;
    };

    let missing: Vec<&&str> = MIN_VIABLE_PRX_STEMS
        .iter()
        .filter(|stem| !dir.join(format!("{stem}.sprx")).is_file())
        .collect();
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter().map(|s| **s).collect();
        if std::env::var_os("CELLGOV_REQUIRE_FIRMWARE_SET_LOAD").is_some() {
            panic!("CELLGOV_REQUIRE_FIRMWARE_SET_LOAD set but PRX stems missing: {names:?}");
        }
        eprintln!("firmware_set_load: skipping (PRX stems missing: {names:?})");
        return;
    }

    let mut bytes_by_path: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut multi_seg_skipped = 0usize;
    for stem in MIN_VIABLE_PRX_STEMS {
        let sprx_path = dir.join(format!("{stem}.sprx"));
        let raw = std::fs::read(&sprx_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", sprx_path.display()));
        let elf = cellgov_firmware::sce::decrypt_self_to_elf(&raw)
            .unwrap_or_else(|e| panic!("decrypt {}: {e}", sprx_path.display()));
        match check_loadable(&elf) {
            Ok(()) => {}
            Err(PrxLoaderError::MultiSegmentRelocations {
                module,
                segment_idx,
            }) => {
                eprintln!(
                    "firmware_set_load: skipping {} ({:?}, segment_idx={segment_idx}) -- multi-segment, deferred phase",
                    sprx_path.display(),
                    module
                );
                multi_seg_skipped += 1;
                continue;
            }
            Err(e) => panic!("check_loadable {}: {e:?}", sprx_path.display()),
        }
        // Firmware filenames are ASCII alphanumeric per install
        // conventions (the reloc-census regenerator enforces the
        // [A-Za-z0-9._-]+ charset on the same directory). Using
        // to_str avoids the lossy fallback that BTreeMap key
        // collisions could exploit.
        let path_str = sprx_path
            .to_str()
            .unwrap_or_else(|| panic!("non-utf8 firmware path: {}", sprx_path.display()))
            .to_string();
        bytes_by_path.insert(path_str, elf);
    }
    assert_eq!(
        multi_seg_skipped, MULTI_SEG_EXPECTED,
        "multi_seg_skipped diverged from the calibrated MULTI_SEG_EXPECTED; either a regression changed the loadable subset or the constant needs recalibration"
    );
    assert_eq!(
        bytes_by_path.len() + multi_seg_skipped,
        MIN_VIABLE_PRX_STEMS.len(),
        "missing modules unaccounted for"
    );
    eprintln!(
        "firmware_set_load: prepared {} minimum-viable PRX modules ({multi_seg_skipped} multi-segment-skipped)",
        bytes_by_path.len()
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

    // (b) Export-table == union of every module's exports.
    let union: BTreeSet<u32> = image
        .loaded
        .values()
        .flat_map(|p| p.exports.keys().copied())
        .collect();
    let table_keys: BTreeSet<u32> = image.export_table.nids().collect();
    assert_eq!(
        union, table_keys,
        "export table != union of per-module exports"
    );
    for prx in image.loaded.values() {
        for (&nid, &opd) in &prx.exports {
            let table_opd = image.export_table.get(nid).expect("nid in table");
            assert_eq!(
                table_opd, opd,
                "table NID 0x{nid:08x} maps to 0x{table_opd:x} but module {} carries 0x{opd:x}",
                prx.name
            );
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
        "firmware_set_load: loaded {} minimum-viable PRX modules; export table {} NIDs",
        image.loaded.len(),
        image.export_table.len()
    );
}

/// Static counterpart to the runtime `host_invariant_breaks`
/// reading: union of the 15-stem set's export namespaces must
/// contain every namespace any title in the corpus imports via
/// the unresolved-trampoline path.
#[test]
fn min_viable_prx_set_exports_required_namespaces() {
    let Some(dir) = locate_firmware_dir() else {
        return;
    };

    let mut namespaces: BTreeSet<String> = BTreeSet::new();
    let mut missing_stems: Vec<&str> = Vec::new();
    for stem in MIN_VIABLE_PRX_STEMS {
        let path = dir.join(format!("{stem}.sprx"));
        if !path.is_file() {
            missing_stems.push(*stem);
            continue;
        }
        let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let elf = cellgov_firmware::sce::decrypt_self_to_elf(&raw)
            .unwrap_or_else(|e| panic!("decrypt {}: {e}", path.display()));
        let parsed = cellgov_ppu::sprx::parse_prx(&elf)
            .unwrap_or_else(|e| panic!("parse_prx {}: {e:?}", path.display()));
        for lib in &parsed.exports {
            namespaces.insert(lib.name.clone());
        }
    }
    if !missing_stems.is_empty() {
        if std::env::var_os("CELLGOV_REQUIRE_FIRMWARE_SET_LOAD").is_some() {
            panic!("CELLGOV_REQUIRE_FIRMWARE_SET_LOAD set but stems missing: {missing_stems:?}");
        }
        eprintln!(
            "min_viable_prx_set_exports_required_namespaces: namespace union built from \
             {}/{} stems (missing: {missing_stems:?})",
            MIN_VIABLE_PRX_STEMS.len() - missing_stems.len(),
            MIN_VIABLE_PRX_STEMS.len(),
        );
    }

    // Namespaces titles in the corpus import via the
    // unresolved-trampoline path; every one must be exported by
    // some stem in the loaded set.
    const REQUIRED: &[&str] = &[
        "cellSysmodule",
        "cellSysutil",
        "cellGcmSys",
        "cellSpurs",
        "sys_io",
        "cellSysutilAvconfExt",
    ];
    for ns in REQUIRED {
        assert!(
            namespaces.contains(*ns),
            "{ns}: namespace identified as a contamination source by the \
             closure investigation is not exported by the {n}-stem firmware set; \
             closure-walk regression",
            n = MIN_VIABLE_PRX_STEMS.len()
        );
    }
    eprintln!(
        "min_viable_prx_set_exports_required_namespaces: {} namespaces exported by {}-stem set, \
         all {} required namespaces present",
        namespaces.len(),
        MIN_VIABLE_PRX_STEMS.len(),
        REQUIRED.len()
    );
}
