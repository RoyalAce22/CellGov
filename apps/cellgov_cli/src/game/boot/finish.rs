//! The debug patches and dumps that land after every `module_start`,
//! and the invariants the boot checks before handing the runtime to
//! the step loop.

use cellgov_core::Runtime;

use super::module_start::ModuleStartCounts;
use super::types::PrepareOptions;
use crate::cli::exit::die;

/// Apply `--patch-byte` writes.
///
/// Runs after `module_start` so a patch overrides the same memory the
/// title sees rather than a value an init routine later rewrites.
pub(super) fn apply_patch_bytes(rt: &mut Runtime, opts: &PrepareOptions<'_>) {
    for &(addr, val) in opts.patch_bytes {
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 1)
            .unwrap_or_else(|| die(&format!("patch: byte 0x{addr:x}: invalid address range")));
        rt.memory_mut()
            .apply_commit(range, &[val])
            .unwrap_or_else(|e| {
                die(&format!(
                    "patch: byte 0x{addr:x} = 0x{val:02x} FAILED ({e:?}); target not committed"
                ))
            });
        if opts.print_banner {
            println!("patch: byte 0x{addr:x} = 0x{val:02x}");
        }
    }
}

/// Hex-dump 32 bytes at each `--dump-mem` address, after
/// `module_start` so the dump observes the memory the title sees.
pub(super) fn dump_boot_memory(rt: &Runtime, addrs: &[u64]) {
    for &addr in addrs {
        match cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 32) {
            None => println!("mem[0x{addr:x}]: invalid address range"),
            Some(r) => match rt.memory().read(r) {
                Some(slice) => {
                    let label = rt
                        .memory()
                        .containing_region(addr, 32)
                        .map(|r| r.label())
                        .unwrap_or("<unmapped>");
                    print!("mem[0x{addr:x}] ({label}):");
                    for b in slice {
                        print!(" {b:02x}");
                    }
                    println!();
                }
                None => println!("mem[0x{addr:x}]: unmapped"),
            },
        }
    }
}

/// Every module the loop was given is accounted for as started or
/// faulted.
///
/// A faulted start is a witnessed skip (`BENCH_MODULE_START_FAULTS`),
/// so it counts toward completeness; `CELLGOV_SKIP_MODULE_START` runs
/// nothing, so the invariant binds only when the loop ran.
pub(super) fn assert_module_start_completeness(counts: &ModuleStartCounts) {
    if counts.skipped {
        return;
    }
    debug_assert_eq!(
        counts.started + counts.faulted,
        counts.total,
        "module_start: completed {} + faulted {} of {} modules",
        counts.started,
        counts.faulted,
        counts.total,
    );
}

/// Liblv2's once-mutex slot.
const LIBLV2_ONCE_MUTEX_SLOT: u64 = 0x103a49d8;

/// If memory holds a non-zero once-mutex id, that id must exist in
/// the LV2 host's mutex table.
pub(super) fn assert_gating_state_coherent_with_host(rt: &Runtime, modules_were_loaded: bool) {
    if !modules_were_loaded {
        return;
    }
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(LIBLV2_ONCE_MUTEX_SLOT), 4)
        .expect("static address range");
    let Some(bytes) = rt.memory().read(range) else {
        return;
    };
    let mutex_id = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if mutex_id == 0 {
        return;
    }
    debug_assert!(
        rt.lv2_host().mutexes().lookup(mutex_id).is_some(),
        "lv2 host handoff witness: liblv2's once-mutex slot at 0x{:016x} references \
         mutex id 0x{:08x} but the host has no such entry",
        LIBLV2_ONCE_MUTEX_SLOT,
        mutex_id,
    );
}

#[cfg(test)]
#[path = "tests/finish_tests.rs"]
mod tests;
