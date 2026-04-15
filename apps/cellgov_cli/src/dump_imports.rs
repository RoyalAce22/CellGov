//! `dump-imports` subcommand: read a title's EBOOT, parse its HLE
//! import table, and print a markdown inventory of every bound
//! function alongside its NID-DB name, stub classification, and
//! whether CellGov has dedicated handling.
//!
//! The regenerated artifacts live under `docs/titles/`:
//!
//! - `docs/titles/flow_hle_inventory.md`
//! - `docs/titles/sshd_hle_inventory.md`
//!
//! Regenerate one with:
//!
//! ```text
//! cellgov_cli dump-imports --title <name> \
//!     > docs/titles/<name>_hle_inventory.md
//! ```
//!
//! Title resolution reuses the run-game / bench-boot VFS lookup
//! path (`--vfs-root`, `$CELLGOV_PS3_VFS_ROOT`,
//! `tools/rpcs3/dev_hdd0`).

use crate::game;

/// NIDs with dedicated CellGov HLE handling, kept as HLE
/// trampolines instead of being patched to a loaded PRX's
/// implementation. Mirrors the list in
/// `game::prx::load_firmware_prx`; the inventory artifact is a
/// diagnostic surface, not a runtime dependency, so the
/// duplication keeps library crates free of CLI-tool coupling.
const CELLGOV_HLE_IMPLEMENTED_NIDS: &[u32] = &[
    0x744680a2, // sys_initialize_tls
    0xbdb18f83, // _sys_malloc
    0xf7f7fb20, // _sys_free
    0x68b9b011, // _sys_memset
    0xe6f2c1e7, // sys_process_exit
    0xb2fcf2c8, // _sys_heap_create_heap
    0x2f85c0ef, // sys_lwmutex_create
    0x1573dc3f, // sys_lwmutex_lock
    0xc3476d0c, // sys_lwmutex_destroy
    0x1bc200f4, // sys_lwmutex_unlock
    0xaeb78725, // sys_lwmutex_trylock
    0x8461e528, // sys_time_get_system_time
    0x350d454e, // sys_ppu_thread_get_id
    0x24a1ea07, // sys_ppu_thread_create
    0x4f7172c9, // sys_process_is_stack
    0xa2c7ba64, // sys_prx_exitspawn_with_level
];

/// Entry point for `cellgov_cli dump-imports --title <name>`.
pub(crate) fn run(args: &[String]) {
    let title = match game::titles::Title::parse_from_args(args) {
        Ok(t) => t,
        Err(game::titles::TitleError::Missing) => {
            eprintln!("dump-imports: --title is required");
            std::process::exit(1);
        }
        Err(game::titles::TitleError::Unknown(v)) => {
            eprintln!("dump-imports: unknown title '{v}'");
            std::process::exit(1);
        }
    };
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
}
