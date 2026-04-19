//! `dump-imports` subcommand: read a title's EBOOT, parse its HLE
//! import table, and print a markdown inventory of every bound
//! function alongside its NID-DB name, stub classification, and
//! whether CellGov has dedicated handling.
//!
//! The regenerated artifacts live under `docs/titles/`, one file
//! per title keyed by PSN content id or disc serial (e.g.
//! `docs/titles/<SERIAL>_hle_inventory.md`).
//!
//! Regenerate one with:
//!
//! ```text
//! cellgov_cli dump-imports --title <shortname> \
//!     > docs/titles/<content-id>_hle_inventory.md
//! ```
//!
//! Title resolution accepts any of `--title <shortname>`,
//! `--content-id <NPUAXXXXX>`, or `--title-manifest <path>`; the
//! VFS lookup path is the same as `run-game` / `bench-boot`
//! (`--vfs-root`, `$CELLGOV_PS3_VFS_ROOT`,
//! `tools/rpcs3/dev_hdd0`).

/// NIDs with dedicated CellGov HLE handling. Re-exported from
/// `cellgov_ppu::prx::HLE_IMPLEMENTED_NIDS` so the inventory tool
/// and the runtime PRX binder read from a single source of truth;
/// adding a new HLE implementation only requires updating the
/// library constant.
const CELLGOV_HLE_IMPLEMENTED_NIDS: &[u32] = cellgov_ppu::prx::HLE_IMPLEMENTED_NIDS;

/// Column width for the Name field in the markdown table. Names
/// longer than this get truncated with a trailing "...". Padding
/// alone (the old `{:<49}` format) did not truncate, so a long
/// mangled C++ symbol would blow the column and misalign the
/// downstream cells in any markdown renderer that honors pipes.
const NAME_COLUMN_WIDTH: usize = 49;

/// Truncate `name` to at most `NAME_COLUMN_WIDTH` chars by keeping
/// the head and appending "..." when it overflows. Char-aware so a
/// non-ASCII byte in a demangled C++ symbol cannot land the cut
/// mid-codepoint and panic the tool. Padding is left to the
/// formatter.
///
/// Caveat: the Rust format-width specifier (`{:<width$}`) pads by
/// byte count, not by display width. Combining sequences or
/// wide-glyph characters in the NID DB will still misalign the
/// table because the formatter counts one byte per display slot.
/// PS3 NID DB entries are overwhelmingly mangled C identifiers
/// or sys_ / cell* prefixes, so in practice this is theoretical;
/// the comment exists so the next person reading this does not
/// chase a phantom alignment bug.
fn fit_name_column(name: &str) -> String {
    if name.chars().count() <= NAME_COLUMN_WIDTH {
        name.to_string()
    } else {
        // Reserve 3 chars for the ellipsis marker. Iterate chars
        // rather than byte-slice so we cannot cut mid-codepoint.
        let head: String = name.chars().take(NAME_COLUMN_WIDTH - 3).collect();
        format!("{head}...")
    }
}

/// Classification result for a single imported NID. Mutually
/// exclusive buckets; each function lands in exactly one. Totals
/// across the run must sum to `total_functions` -- the invariant
/// is asserted at the end of `run`, so a future refactor that
/// double-counts or forgets a bucket fails loudly instead of
/// producing a silently wrong "noop-safe" count via unsigned
/// subtraction underflow.
#[derive(Debug, Default, Clone, Copy)]
struct InventoryCounts {
    /// NID is in CELLGOV_HLE_IMPLEMENTED_NIDS; dispatch has
    /// dedicated handling.
    impl_count: usize,
    /// NID is not implemented and classified "stateful"; needs a
    /// real impl, stub to 0 would be wrong.
    stateful_unstubbed: usize,
    /// NID is not implemented and classified "unsafe-to-stub";
    /// stub to 0 may cause incorrect behavior.
    unsafe_unstubbed: usize,
    /// NID is not implemented and classified "noop-safe"; stub to
    /// 0 is fine.
    noop_safe: usize,
    /// NID is not in the NID DB at all. We have no name, cannot
    /// classify, and must not assume the default stub is safe.
    /// Kept in its own bucket so operators see unknowns instead of
    /// having them folded into the "noop-safe" count.
    unknown_nid: usize,
    /// NID is in the DB but `stub_classification` returned a
    /// string we did not expect. A library-side schema change
    /// would otherwise get silently reclassified as noop-safe by
    /// the old subtraction arithmetic.
    unknown_class: usize,
}

impl InventoryCounts {
    fn sum(&self) -> usize {
        self.impl_count
            + self.stateful_unstubbed
            + self.unsafe_unstubbed
            + self.noop_safe
            + self.unknown_nid
            + self.unknown_class
    }

    fn bump(&mut self, bucket: Bucket) {
        match bucket {
            Bucket::Impl => self.impl_count += 1,
            Bucket::Stateful => self.stateful_unstubbed += 1,
            Bucket::UnsafeToStub => self.unsafe_unstubbed += 1,
            Bucket::NoopSafe => self.noop_safe += 1,
            Bucket::UnknownNid => self.unknown_nid += 1,
            Bucket::UnknownClass => self.unknown_class += 1,
        }
    }
}

/// Disjoint classification outcomes; `InventoryCounts` has one
/// field per variant. Adding a new bucket means one new arm in
/// `classify_import` and one in `InventoryCounts::bump`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bucket {
    Impl,
    Stateful,
    UnsafeToStub,
    NoopSafe,
    UnknownNid,
    UnknownClass,
}

/// Single decision point: given a NID plus its nid_db lookup
/// result, pick a bucket and the string that belongs in the Class
/// column. Previously the counter increment and the cell text came
/// from two separate matches, which is the same "two places must
/// agree" shape the original file got wrong. One function, one
/// truth.
fn classify_import(
    nid: u32,
    lookup: Option<(&'static str, &'static str)>,
) -> (Bucket, &'static str, bool) {
    if lookup.is_none() {
        // Invariant asserted by the `every_implemented_nid_is_in_nid_db`
        // test: any NID the runtime dispatch handles must also be
        // in nid_db. If that ever regresses, this branch would
        // silently render an impl NID as `stub <unknown-nid>`; the
        // debug_assert turns the first dev-run into a loud failure.
        debug_assert!(
            !CELLGOV_HLE_IMPLEMENTED_NIDS.contains(&nid),
            "NID 0x{nid:08x} is in HLE_IMPLEMENTED_NIDS but nid_db has no entry -- \
             every_implemented_nid_is_in_nid_db test must have been skipped or regressed"
        );
        return (Bucket::UnknownNid, "<unknown-nid>", false);
    }
    let class_str = cellgov_ppu::nid_db::stub_classification(nid);
    let is_impl = CELLGOV_HLE_IMPLEMENTED_NIDS.contains(&nid);
    let (bucket, class_cell) = match (is_impl, class_str) {
        (true, "stateful") => (Bucket::Impl, "stateful"),
        (true, "unsafe-to-stub") => (Bucket::Impl, "unsafe-to-stub"),
        (true, "noop-safe") => (Bucket::Impl, "noop-safe"),
        (true, _) => (Bucket::Impl, "<unknown-class>"),
        (false, "stateful") => (Bucket::Stateful, "stateful"),
        (false, "unsafe-to-stub") => (Bucket::UnsafeToStub, "unsafe-to-stub"),
        (false, "noop-safe") => (Bucket::NoopSafe, "noop-safe"),
        (false, _) => (Bucket::UnknownClass, "<unknown-class>"),
    };
    (bucket, class_cell, is_impl)
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

    // Zero-module output is almost always the wrong artifact to
    // commit over a previously-correct inventory. Exit non-zero so
    // a regenerate-in-CI loop does not silently overwrite a real
    // doc with an empty one (ELF stripped of imports, not-a-PRX
    // binary, parser returning Ok(vec![]) on a near-miss).
    if modules.is_empty() {
        eprintln!(
            "dump-imports: parse_imports returned zero modules for {}; refusing to produce an empty inventory",
            elf_path.display()
        );
        std::process::exit(1);
    }

    println!("# {} HLE Import Inventory", title.display_name());
    println!();
    // Path rendering: use to_str() first so a round-trippable form
    // lands in the doc. On non-UTF-8 paths (possible on Linux with
    // exotic filenames) to_string_lossy substitutes U+FFFD and the
    // rendered path cannot be used to re-open the file. Flag that
    // to stderr so an operator pasting the path from the doc is
    // not silently confused.
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
    println!("  NID is not in `cellgov_ppu::nid_db` at all (no symbol");
    println!("  available).");
    println!("- **Class**: `stub_classification(nid)` from the NID DB.");
    println!("  `stateful` / `unsafe-to-stub` need real impls; `noop-safe`");
    println!("  is fine returning 0. `<unknown-nid>` distinguishes the");
    println!("  case where the NID itself is missing from the DB (same");
    println!("  condition that shows `<unknown>` in the Name column);");
    println!("  `<unknown-class>` means the NID is in the DB but the");
    println!("  classifier returned a string this tool does not recognize");
    println!("  (library-side schema drift).");
    println!("- **CellGov**: `impl` if the NID has dedicated handling in");
    println!("  `cellgov_core::hle::dispatch_hle` or the HLE-keep list in");
    println!("  `game::prx::load_firmware_prx`; `stub` otherwise (default");
    println!("  returns 0).");
    println!();

    let mut counts = InventoryCounts::default();
    // Suspected collisions: binding's module (from the game's ELF
    // import table) disagrees with the nid_db's module for the
    // same NID. NIDs are hashes and collisions across modules
    // have happened historically in PS3 tooling; surface the
    // mismatch so a mis-attributed "impl" mark is not silent.
    //
    // Exact-string comparison caveat: PS3 module names have
    // historical variants (versioned stubs, trailing-underscore
    // aliases, internal-init entrypoints) so this check can fire
    // false positives on titles that import via a variant the
    // nid_db does not normalize. If that becomes a flood, the
    // expected fix is an allow-list of known-equivalent name
    // pairs or a `--ignore-module-name-mismatch` flag -- not a
    // reason to relax the default.
    let mut module_collisions: Vec<(String, u32, String, String)> = Vec::new();
    let mut empty_modules: Vec<String> = Vec::new();

    for module in &modules {
        if module.functions.is_empty() {
            // Emit a strange empty section into the doc is worse
            // than omitting it; record the name for a stderr note
            // instead of producing a zero-row table that looks
            // like a parse glitch to the next reviewer.
            empty_modules.push(module.name.clone());
            continue;
        }
        println!("## {} ({} functions)", module.name, module.functions.len());
        println!();
        println!("| NID        | Name                                              | Class           | CellGov |");
        println!("|------------|---------------------------------------------------|-----------------|---------|");
        for f in &module.functions {
            let lookup = cellgov_ppu::nid_db::lookup(f.nid);
            let name: &str = lookup.map(|(_m, n)| n).unwrap_or("<unknown>");
            let (bucket, class_cell, is_impl) = classify_import(f.nid, lookup);
            counts.bump(bucket);

            // Collision cross-check runs only for bindings marked
            // impl -- there is no ambiguity to flag when dispatch
            // does not handle the NID at all. Keeps the check
            // close to where the mis-attribution would matter.
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

    // Flush every collected diagnostic to stderr BEFORE the
    // counter invariant check. If the invariant panics, the
    // operator most needs the collision and empty-module notes
    // to diagnose what happened -- emitting them after a panic
    // loses them. Summary stdout print is after the assert so
    // the happy path keeps the tidy "data, then advisories"
    // ordering on stderr/stdout split consumers.
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
    // Panics (even in release) are appropriate here because the
    // summary arithmetic feeds operator decisions about what is
    // safe to leave stubbed. A double-count or miss is a real bug
    // to surface, not a warning to tolerate.
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
    if counts.unknown_class > 0 {
        println!(
            "- Unknown classification (schema drift?): {}",
            counts.unknown_class
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implemented_nids_is_nonempty_and_contains_tls_init() {
        // Every PS3 ELF boot calls sys_initialize_tls; dropping the
        // NID from this list silently downgrades the inventory for
        // every title.
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

    /// Every NID marked "CellGov-implemented" must exist in the
    /// NID DB. A NID that dispatch handles but nid_db does not
    /// know would render as `impl  <unknown>` in every inventory;
    /// a typo'd NID in the list would never match a real import
    /// and would quietly stay there forever.
    #[test]
    fn every_implemented_nid_is_in_nid_db() {
        for &nid in CELLGOV_HLE_IMPLEMENTED_NIDS {
            assert!(
                cellgov_ppu::nid_db::lookup(nid).is_some(),
                "NID 0x{nid:08x} is in HLE_IMPLEMENTED_NIDS but absent from nid_db"
            );
        }
    }

    /// Every NID marked implemented must classify as one of the
    /// three expected strings. A library-side rename or new class
    /// would otherwise land in the tool's `unknown_class` bucket
    /// at runtime, which is a real regression a test can catch
    /// before any inventory regenerates.
    #[test]
    fn every_implemented_nid_classifies_to_known_string() {
        for &nid in CELLGOV_HLE_IMPLEMENTED_NIDS {
            let c = cellgov_ppu::nid_db::stub_classification(nid);
            assert!(
                matches!(c, "stateful" | "unsafe-to-stub" | "noop-safe"),
                "NID 0x{nid:08x} classified as unexpected string {c:?}"
            );
        }
    }

    #[test]
    fn fit_name_column_passes_short_names_unchanged() {
        assert_eq!(fit_name_column("short"), "short");
        // Exactly at the width stays intact.
        let exact = "x".repeat(NAME_COLUMN_WIDTH);
        assert_eq!(fit_name_column(&exact), exact);
    }

    #[test]
    fn fit_name_column_truncates_overlong_names_with_ellipsis() {
        let long = "x".repeat(NAME_COLUMN_WIDTH + 50);
        let fit = fit_name_column(&long);
        // Assert char count, not byte length: the char-aware cut
        // keeps the invariant on graphemes, not bytes.
        assert_eq!(fit.chars().count(), NAME_COLUMN_WIDTH);
        assert!(fit.ends_with("..."));
    }

    #[test]
    fn fit_name_column_does_not_panic_on_multibyte_input() {
        // Non-ASCII characters in a NID DB entry (demangled C++
        // symbols can legitimately contain them) must not panic
        // the tool. Case 1: a bulk-multibyte string exercises the
        // char-aware cut generally.
        let long_multibyte: String = "ab\u{00e9}".repeat(30); // 90 chars, 120 bytes
        let fit = fit_name_column(&long_multibyte);
        assert_eq!(fit.chars().count(), NAME_COLUMN_WIDTH);
        assert!(fit.ends_with("..."));
    }

    #[test]
    fn fit_name_column_handles_codepoint_straddling_the_cut_boundary() {
        // The cut index is NAME_COLUMN_WIDTH - 3. Construct a
        // string where a two-byte codepoint starts at exactly that
        // position, so the old byte-slicing implementation
        // `&name[..NAME_COLUMN_WIDTH - 3]` would panic with
        // "byte index is not a char boundary". The char-aware
        // cut via `chars().take(N)` succeeds and preserves char
        // count invariants.
        let cut = NAME_COLUMN_WIDTH - 3;
        let prefix = "a".repeat(cut); // ASCII padding up to the boundary
        let straddler = "\u{00e9}"; // 2-byte UTF-8 starting at byte `cut`
        let tail = "z".repeat(100); // pad past the width
        let input = format!("{prefix}{straddler}{tail}");
        // Sanity: the straddling codepoint does begin at byte
        // index `cut`, confirming the byte-slice path would
        // have failed here.
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
            unknown_class: 6,
        };
        assert_eq!(c.sum(), 21);
    }

    #[test]
    fn classify_import_routes_unknown_nid_to_unknown_bucket() {
        // A NID that is not in the DB must land in UnknownNid,
        // not silently flow through stub_classification's
        // _ => "noop-safe" fallthrough.
        let (bucket, cell, is_impl) = classify_import(0xDEAD_BEEF, None);
        assert_eq!(bucket, Bucket::UnknownNid);
        assert_eq!(cell, "<unknown-nid>");
        assert!(!is_impl);
    }

    #[test]
    fn classify_import_routes_implemented_nid_to_impl_bucket() {
        // sys_initialize_tls: present in nid_db and in the
        // implemented list, classified "stateful".
        let nid = 0x744680a2;
        let lookup = cellgov_ppu::nid_db::lookup(nid);
        assert!(lookup.is_some(), "test precondition: nid_db knows this NID");
        let (bucket, cell, is_impl) = classify_import(nid, lookup);
        assert_eq!(bucket, Bucket::Impl);
        assert_eq!(cell, "stateful");
        assert!(is_impl);
    }

    #[test]
    fn classify_import_routes_noop_safe_nid_while_sce_np_remains_unimplemented() {
        // Purpose: exercise the NoopSafe arm of classify_import.
        // Picks a real NID (sceNpManagerSubSignout) that is in
        // nid_db, classifies as noop-safe, and is not in the
        // implemented list today. This test name flags the
        // assumption: if someone adds this NID to
        // HLE_IMPLEMENTED_NIDS or the NID DB retags it, the
        // precondition below fires first with a message telling
        // the reader to pick a different noop-safe witness.
        let nid = 0x000e53cc; // sceNpManagerSubSignout
        assert!(
            !CELLGOV_HLE_IMPLEMENTED_NIDS.contains(&nid),
            "precondition drifted: 0x{nid:08x} is now implemented; swap to another noop-safe NID"
        );
        let lookup = cellgov_ppu::nid_db::lookup(nid);
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
        // Walking every variant and asserting sum tracks catches
        // a future refactor where a new bucket is added to the
        // enum and to classify_import but not to bump().
        let mut c = InventoryCounts::default();
        for bucket in [
            Bucket::Impl,
            Bucket::Stateful,
            Bucket::UnsafeToStub,
            Bucket::NoopSafe,
            Bucket::UnknownNid,
            Bucket::UnknownClass,
        ] {
            c.bump(bucket);
        }
        assert_eq!(c.sum(), 6);
    }
}
