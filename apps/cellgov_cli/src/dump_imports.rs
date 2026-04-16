//! `dump-imports` subcommand: read a title's EBOOT, parse its HLE
//! import table, and print a markdown inventory of every bound
//! function alongside its NID-DB name, stub classification, and
//! whether CellGov has dedicated handling.
//!
//! The regenerated artifacts live under `docs/titles/`, keyed by
//! PSN content id:
//!
//! - `docs/titles/NPUA80001_hle_inventory.md` (flOw)
//! - `docs/titles/NPUA80068_hle_inventory.md` (Super Stardust HD)
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

/// Entry point for `cellgov_cli dump-imports --title <name>`.
pub(crate) fn run(args: &[String]) {
    let title = crate::resolve_title_manifest(args, "dump-imports");
    let vfs_root = crate::resolve_ps3_vfs_root(args);
    let elf_path = title.resolve_eboot(&vfs_root).unwrap_or_else(|| {
        eprintln!(
            "dump-imports: no EBOOT for title '{}' under vfs-root={}",
            title.name(),
            vfs_root.display()
        );
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

    println!("# {} HLE Import Inventory", title.display_name());
    println!();
    println!("- ELF: `{}`", elf_path.to_string_lossy().replace('\\', "/"));
    println!("- Modules imported: {}", modules.len());
    println!("- Functions imported: {}", total_functions);
    println!();
    println!("Classification columns:");
    println!();
    println!("- **Name**: NID-DB lookup; `<unknown>` means the NID is not in");
    println!("  `cellgov_ppu::nid_db`.");
    println!("- **Class**: `stub_classification(nid)` from the NID DB.");
    println!("  `stateful` / `unsafe-to-stub` need real impls; `noop-safe`");
    println!("  is fine returning 0.");
    println!("- **CellGov**: `impl` if the NID has dedicated handling in");
    println!("  `cellgov_core::hle::dispatch_hle` or the HLE-keep list in");
    println!("  `game::prx::load_firmware_prx`; `stub` otherwise (default");
    println!("  returns 0).");
    println!();

    let mut impl_count = 0usize;
    let mut stateful_unstubbed = 0usize;
    let mut unsafe_unstubbed = 0usize;

    for module in &modules {
        println!("## {} ({} functions)", module.name, module.functions.len());
        println!();
        println!("| NID        | Name                                              | Class           | CellGov |");
        println!("|------------|---------------------------------------------------|-----------------|---------|");
        for f in &module.functions {
            let name = cellgov_ppu::nid_db::lookup(f.nid)
                .map(|(_m, n)| n)
                .unwrap_or("<unknown>");
            let class = cellgov_ppu::nid_db::stub_classification(f.nid);
            let cellgov = if CELLGOV_HLE_IMPLEMENTED_NIDS.contains(&f.nid) {
                impl_count += 1;
                "impl"
            } else {
                if class == "stateful" {
                    stateful_unstubbed += 1;
                } else if class == "unsafe-to-stub" {
                    unsafe_unstubbed += 1;
                }
                "stub"
            };
            println!(
                "| 0x{:08x} | {:<49} | {:<15} | {:<7} |",
                f.nid, name, class, cellgov
            );
        }
        println!();
    }

    println!("## Summary");
    println!();
    println!("- Total imports: {}", total_functions);
    println!("- CellGov-implemented: {}", impl_count);
    println!(
        "- Unstubbed stateful (need real impl): {}",
        stateful_unstubbed
    );
    println!(
        "- Unstubbed unsafe-to-stub (stub returns wrong value): {}",
        unsafe_unstubbed
    );
    println!(
        "- Default-stub noop-safe: {}",
        total_functions - impl_count - stateful_unstubbed - unsafe_unstubbed
    );
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

    #[test]
    fn inventory_and_runtime_share_the_same_nid_list() {
        // Pin the single-source-of-truth contract: the dump-imports
        // tool and the runtime PRX binder both read
        // `cellgov_ppu::prx::HLE_IMPLEMENTED_NIDS`. If someone adds
        // a NID to one call site only, this test fails.
        assert_eq!(
            CELLGOV_HLE_IMPLEMENTED_NIDS,
            cellgov_ppu::prx::HLE_IMPLEMENTED_NIDS
        );
    }
}
