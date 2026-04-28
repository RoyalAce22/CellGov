//! `dump-imports` subcommand: parse a title's EBOOT import table and
//! print a markdown inventory bucketed by classification.
//!
//! Each imported NID falls into exactly one of five buckets:
//! `Impl` (CellGov has dedicated handling), `Stateful` /
//! `UnsafeToStub` / `NoopSafe` (stub-safety tier from the NID DB),
//! or `UnknownNid` (not in the DB). The buckets drive the summary
//! arithmetic at the end of `run`. The classifier returns a typed
//! `StubClass` so a typo cannot create an unknown-class drift bucket.
//!
//! Artifacts land in `docs/titles/<content-id>_hle_inventory.md`.

/// NIDs with dedicated CellGov HLE handling. Shared with the runtime
/// PRX binder; adding an HLE impl means updating this library constant.
const CELLGOV_HLE_IMPLEMENTED_NIDS: &[u32] = cellgov_ppu::prx::HLE_IMPLEMENTED_NIDS;

/// Column width for the Name field in the markdown table.
const NAME_COLUMN_WIDTH: usize = 49;

/// Char-aware truncation to `NAME_COLUMN_WIDTH`, appending "..." on
/// overflow. Padding is left to the format specifier.
///
/// Caveat: `{:<width$}` pads by byte count, so combining sequences
/// or wide glyphs still misalign the table. PS3 NID DB entries are
/// overwhelmingly ASCII C identifiers, so this is theoretical.
fn fit_name_column(name: &str) -> String {
    if name.chars().count() <= NAME_COLUMN_WIDTH {
        name.to_string()
    } else {
        // chars().take to avoid cutting mid-codepoint.
        let head: String = name.chars().take(NAME_COLUMN_WIDTH - 3).collect();
        format!("{head}...")
    }
}

/// Per-bucket counters. Sum across all fields equals
/// `total_functions`; `run` asserts the invariant.
#[derive(Debug, Default, Clone, Copy)]
struct InventoryCounts {
    impl_count: usize,
    stateful_unstubbed: usize,
    unsafe_unstubbed: usize,
    noop_safe: usize,
    unknown_nid: usize,
}

impl InventoryCounts {
    fn sum(&self) -> usize {
        self.impl_count
            + self.stateful_unstubbed
            + self.unsafe_unstubbed
            + self.noop_safe
            + self.unknown_nid
    }

    fn bump(&mut self, bucket: Bucket) {
        match bucket {
            Bucket::Impl => self.impl_count += 1,
            Bucket::Stateful => self.stateful_unstubbed += 1,
            Bucket::UnsafeToStub => self.unsafe_unstubbed += 1,
            Bucket::NoopSafe => self.noop_safe += 1,
            Bucket::UnknownNid => self.unknown_nid += 1,
        }
    }
}

/// Disjoint classification outcomes; `InventoryCounts` has one field
/// per variant, and `classify_import` returns one per NID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bucket {
    Impl,
    Stateful,
    UnsafeToStub,
    NoopSafe,
    UnknownNid,
}

/// Map a NID and its nid_db lookup to a bucket plus the Class-column
/// string and whether the NID is on the impl list.
fn classify_import(
    nid: u32,
    lookup: Option<(&'static str, &'static str)>,
) -> (Bucket, &'static str, bool) {
    if lookup.is_none() {
        // Hard-assert (not debug-only): an implemented NID missing from
        // nid_db would silently classify as UnknownNid + stub here in
        // release builds -- the worst possible outcome for a function
        // CellGov claims to dispatch. The every_implemented_nid_is_in_nid_db
        // test catches this offline; this guard catches it in any
        // dump-imports invocation regardless of test coverage.
        assert!(
            !CELLGOV_HLE_IMPLEMENTED_NIDS.contains(&nid),
            "NID 0x{nid:08x} is in HLE_IMPLEMENTED_NIDS but nid_db has no entry"
        );
        return (Bucket::UnknownNid, "<unknown-nid>", false);
    }
    use cellgov_ps3_abi::nid::StubClass;
    let class = cellgov_ps3_abi::nid::stub_classification(nid);
    let is_impl = CELLGOV_HLE_IMPLEMENTED_NIDS.contains(&nid);
    let bucket = match (is_impl, class) {
        (true, _) => Bucket::Impl,
        (false, StubClass::Stateful) => Bucket::Stateful,
        (false, StubClass::UnsafeToStub) => Bucket::UnsafeToStub,
        (false, StubClass::NoopSafe) => Bucket::NoopSafe,
    };
    (bucket, class.as_str(), is_impl)
}

/// Entry point for `cellgov_cli dump-imports --title <name>`.
pub(crate) fn run(args: &[String]) {
    let title = crate::cli::title::resolve_title_manifest(args, "dump-imports");
    let vfs_root = crate::cli::title::resolve_ps3_vfs_root(args);
    let elf_path = title.resolve_eboot(&vfs_root).unwrap_or_else(|e| {
        eprintln!("dump-imports: {e}");
        std::process::exit(1);
    });
    let elf_data = std::fs::read(&elf_path).unwrap_or_else(|e| {
        eprintln!("dump-imports: read {}: {e}", elf_path.display());
        std::process::exit(1);
    });
    let modules = cellgov_ppu::prx::parse_imports(&elf_data).unwrap_or_else(|e| {
        eprintln!("dump-imports: parse_imports failed: {e:?}");
        std::process::exit(1);
    });
    let total_functions: usize = modules.iter().map(|m| m.functions.len()).sum();

    // Refuse to overwrite a real inventory with an empty one.
    if modules.is_empty() {
        eprintln!(
            "dump-imports: parse_imports returned zero modules for {}; refusing to produce an empty inventory",
            elf_path.display()
        );
        std::process::exit(1);
    }

    println!("# {} HLE Import Inventory", title.display_name());
    println!();
    // Try to_str first so a round-trippable path lands in the doc;
    // lossy form warns on stderr.
    let path_str: String = match elf_path.to_str() {
        Some(s) => s.replace('\\', "/"),
        None => {
            eprintln!(
                "dump-imports: ELF path is not valid UTF-8; doc will render a lossy form that may not round-trip"
            );
            elf_path.to_string_lossy().replace('\\', "/")
        }
    };
    println!("- ELF: `{path_str}`");
    println!("- Modules imported: {}", modules.len());
    println!("- Functions imported: {total_functions}");
    println!();
    println!("Classification columns:");
    println!();
    println!("- **Name**: NID-DB lookup result. Renders `<unknown>` when the");
    println!("  NID is not in `cellgov_ps3_abi::nid` at all (no symbol");
    println!("  available).");
    println!("- **Class**: `stub_classification(nid)` from the NID DB.");
    println!("  `stateful` / `unsafe-to-stub` need real impls; `noop-safe`");
    println!("  is fine returning 0. `<unknown-nid>` distinguishes the");
    println!("  case where the NID itself is missing from the DB (same");
    println!("  condition that shows `<unknown>` in the Name column).");
    println!("- **CellGov**: `impl` if the NID has dedicated handling in");
    println!("  `cellgov_core::hle::dispatch_hle` or the HLE-keep list in");
    println!("  `game::prx::load_firmware_prx`; `stub` otherwise (default");
    println!("  returns 0).");
    println!();

    let mut counts = InventoryCounts::default();
    // Surface NID/module mismatches: the binding's module disagrees
    // with nid_db's module for the same NID. PS3 NIDs are hashes
    // and cross-module collisions are possible. Exact-string
    // comparison may false-positive on module-name variants
    // (versioned stubs, internal-init entrypoints).
    let mut module_collisions: Vec<(String, u32, String, String)> = Vec::new();
    let mut empty_modules: Vec<String> = Vec::new();

    for module in &modules {
        if module.functions.is_empty() {
            // Omit from the doc; note on stderr.
            empty_modules.push(module.name.clone());
            continue;
        }
        println!("## {} ({} functions)", module.name, module.functions.len());
        println!();
        println!("| NID        | Name                                              | Class           | CellGov |");
        println!("|------------|---------------------------------------------------|-----------------|---------|");
        for f in &module.functions {
            let lookup = cellgov_ps3_abi::nid::lookup(f.nid);
            let name: &str = lookup.map(|(_m, n)| n).unwrap_or("<unknown>");
            let (bucket, class_cell, is_impl) = classify_import(f.nid, lookup);
            counts.bump(bucket);

            // Only cross-check bindings that claim impl status: a
            // mismatch only misattributes the impl mark.
            if is_impl {
                if let Some((db_mod, _)) = lookup {
                    if !db_mod.is_empty() && db_mod != module.name {
                        module_collisions.push((
                            module.name.clone(),
                            f.nid,
                            db_mod.to_string(),
                            name.to_string(),
                        ));
                    }
                }
            }

            let cellgov = if is_impl { "impl" } else { "stub" };
            println!(
                "| 0x{:08x} | {:<width$} | {:<15} | {:<7} |",
                f.nid,
                fit_name_column(name),
                class_cell,
                cellgov,
                width = NAME_COLUMN_WIDTH,
            );
        }
        println!();
    }

    // Flush diagnostics before the counter assertion so a panic
    // does not swallow them.
    if !empty_modules.is_empty() {
        eprintln!(
            "dump-imports: {} module(s) imported with zero functions; omitted from the inventory:",
            empty_modules.len()
        );
        for name in &empty_modules {
            eprintln!("  {name}");
        }
    }
    if !module_collisions.is_empty() {
        eprintln!(
            "dump-imports: {} suspected NID/module collision(s); impl mark may be mis-attributed:",
            module_collisions.len()
        );
        for (binding_mod, nid, db_mod, name) in &module_collisions {
            eprintln!(
                "  nid=0x{nid:08x} imported as {binding_mod} but nid_db resolves to {db_mod}::{name}"
            );
        }
    }

    // Invariant: every function landed in exactly one bucket.
    assert_eq!(
        counts.sum(),
        total_functions,
        "inventory counter invariant broke: sum={} total={}",
        counts.sum(),
        total_functions
    );

    println!("## Summary");
    println!();
    println!("- Total imports: {total_functions}");
    println!("- CellGov-implemented: {}", counts.impl_count);
    println!(
        "- Unstubbed stateful (need real impl): {}",
        counts.stateful_unstubbed
    );
    println!(
        "- Unstubbed unsafe-to-stub (stub returns wrong value): {}",
        counts.unsafe_unstubbed
    );
    println!("- Default-stub noop-safe: {}", counts.noop_safe);
    if counts.unknown_nid > 0 {
        println!(
            "- Unknown NIDs (not in nid_db; safety cannot be assessed): {}",
            counts.unknown_nid
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implemented_nids_is_nonempty_and_contains_tls_init() {
        // sys_initialize_tls is called by every PS3 ELF boot.
        assert!(CELLGOV_HLE_IMPLEMENTED_NIDS.contains(&0x744680a2));
        assert!(CELLGOV_HLE_IMPLEMENTED_NIDS.len() > 4);
    }

    #[test]
    fn implemented_nids_are_unique() {
        let mut sorted = CELLGOV_HLE_IMPLEMENTED_NIDS.to_vec();
        sorted.sort();
        let mut deduped = sorted.clone();
        deduped.dedup();
        assert_eq!(sorted.len(), deduped.len(), "duplicate NID in list");
    }

    #[test]
    fn every_implemented_nid_is_in_nid_db() {
        for &nid in CELLGOV_HLE_IMPLEMENTED_NIDS {
            assert!(
                cellgov_ps3_abi::nid::lookup(nid).is_some(),
                "NID 0x{nid:08x} is in HLE_IMPLEMENTED_NIDS but absent from nid_db"
            );
        }
    }

    #[test]
    fn no_implemented_nid_falls_through_to_default_noop_safe() {
        // Implementation list and stub classifier are kept in sync by
        // hand. Every implemented NID must have an explicit arm in
        // stub_classification; falling through to the catch-all NoopSafe
        // means the classifier silently considers a stateful function
        // safe to leave unstubbed. _sys_free is the lone genuine
        // NoopSafe in the impl list (memory leak is acceptable for
        // triage runs).
        use cellgov_ps3_abi::nid::StubClass;
        const SYS_FREE_NID: u32 = 0xf7f7fb20;
        for &nid in CELLGOV_HLE_IMPLEMENTED_NIDS {
            let c = cellgov_ps3_abi::nid::stub_classification(nid);
            if nid == SYS_FREE_NID {
                assert_eq!(c, StubClass::NoopSafe);
            } else {
                assert!(
                    matches!(c, StubClass::Stateful | StubClass::UnsafeToStub),
                    "NID 0x{nid:08x} fell through to default NoopSafe; \
                     add an explicit StubClass arm to nid_db::stub_classification"
                );
            }
        }
    }

    #[test]
    fn fit_name_column_passes_short_names_unchanged() {
        assert_eq!(fit_name_column("short"), "short");
        let exact = "x".repeat(NAME_COLUMN_WIDTH);
        assert_eq!(fit_name_column(&exact), exact);
    }

    #[test]
    fn fit_name_column_truncates_overlong_names_with_ellipsis() {
        let long = "x".repeat(NAME_COLUMN_WIDTH + 50);
        let fit = fit_name_column(&long);
        assert_eq!(fit.chars().count(), NAME_COLUMN_WIDTH);
        assert!(fit.ends_with("..."));
    }

    #[test]
    fn fit_name_column_does_not_panic_on_multibyte_input() {
        let long_multibyte: String = "ab\u{00e9}".repeat(30);
        let fit = fit_name_column(&long_multibyte);
        assert_eq!(fit.chars().count(), NAME_COLUMN_WIDTH);
        assert!(fit.ends_with("..."));
    }

    #[test]
    fn fit_name_column_handles_codepoint_straddling_the_cut_boundary() {
        // A two-byte codepoint starts exactly at the cut index so a
        // byte-slice implementation would panic on the char-boundary.
        let cut = NAME_COLUMN_WIDTH - 3;
        let prefix = "a".repeat(cut);
        let straddler = "\u{00e9}";
        let tail = "z".repeat(100);
        let input = format!("{prefix}{straddler}{tail}");
        assert_eq!(input.as_bytes()[cut], 0xC3);
        let fit = fit_name_column(&input);
        assert_eq!(fit.chars().count(), NAME_COLUMN_WIDTH);
        assert!(fit.ends_with("..."));
    }

    #[test]
    fn inventory_counts_sum_matches_components() {
        let c = InventoryCounts {
            impl_count: 1,
            stateful_unstubbed: 2,
            unsafe_unstubbed: 3,
            noop_safe: 4,
            unknown_nid: 5,
        };
        assert_eq!(c.sum(), 15);
    }

    #[test]
    fn classify_import_routes_unknown_nid_to_unknown_bucket() {
        let (bucket, cell, is_impl) = classify_import(0xDEAD_BEEF, None);
        assert_eq!(bucket, Bucket::UnknownNid);
        assert_eq!(cell, "<unknown-nid>");
        assert!(!is_impl);
    }

    #[test]
    fn classify_import_routes_implemented_nid_to_impl_bucket() {
        // sys_initialize_tls: stateful and in the implemented list.
        let nid = 0x744680a2;
        let lookup = cellgov_ps3_abi::nid::lookup(nid);
        assert!(lookup.is_some(), "test precondition: nid_db knows this NID");
        let (bucket, cell, is_impl) = classify_import(nid, lookup);
        assert_eq!(bucket, Bucket::Impl);
        assert_eq!(cell, "stateful");
        assert!(is_impl);
    }

    #[test]
    fn classify_import_routes_noop_safe_nid_while_sce_np_remains_unimplemented() {
        // Witness: sceNpManagerSubSignout. If its status drifts, the
        // precondition asserts guide swapping to another noop-safe NID.
        let nid = 0x000e53cc;
        assert!(
            !CELLGOV_HLE_IMPLEMENTED_NIDS.contains(&nid),
            "precondition drifted: 0x{nid:08x} is now implemented; swap to another noop-safe NID"
        );
        let lookup = cellgov_ps3_abi::nid::lookup(nid);
        assert!(
            lookup.is_some(),
            "precondition drifted: 0x{nid:08x} no longer in nid_db; swap to another noop-safe NID"
        );
        let (bucket, cell, is_impl) = classify_import(nid, lookup);
        assert_eq!(bucket, Bucket::NoopSafe);
        assert_eq!(cell, "noop-safe");
        assert!(!is_impl);
    }

    #[test]
    fn bump_agrees_with_sum_for_all_buckets() {
        let mut c = InventoryCounts::default();
        for bucket in [
            Bucket::Impl,
            Bucket::Stateful,
            Bucket::UnsafeToStub,
            Bucket::NoopSafe,
            Bucket::UnknownNid,
        ] {
            c.bump(bucket);
        }
        assert_eq!(c.sum(), 5);
    }
}
