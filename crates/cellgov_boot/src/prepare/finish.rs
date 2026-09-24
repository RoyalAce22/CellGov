//! The debug patches and dumps that land after every `module_start`,
//! the invariants the boot checks before handing the runtime to the
//! step loop, and the title's RSX settings.

use cellgov_core::{AddressSpaceId, Runtime};

use crate::manifest::TitleManifest;

use super::module_start::ModuleStartCounts;
use super::types::{DiagnosticOptions, ExecutionOptions};

/// Why a `--patch-byte` write did not land.
#[derive(Debug, thiserror::Error)]
pub enum PatchError {
    /// The address is not a writable one-byte range.
    #[error("patch: byte 0x{addr:016x}: invalid address range")]
    BadRange {
        /// The address the patch named.
        addr: u64,
    },
    /// The commit pipeline refused the write, so the target keeps its
    /// pre-patch byte.
    #[error("patch: byte 0x{addr:016x} = 0x{value:02x} FAILED ({detail}); target not written")]
    Refused {
        /// The address the patch named.
        addr: u64,
        /// The byte that did not land.
        value: u8,
        /// The commit pipeline's own account of the refusal.
        detail: String,
    },
}

/// Apply `--patch-byte` writes.
///
/// Runs after `module_start` so a patch overrides the same memory the
/// title sees rather than a value an init routine later rewrites.
///
/// # Errors
///
/// An address that is not writable, or a write the commit pipeline
/// refused; see [`PatchError`].
pub(super) fn apply_patch_bytes(
    rt: &mut Runtime,
    execution: &ExecutionOptions<'_>,
    diagnostics: &DiagnosticOptions<'_>,
    sink: &dyn crate::BootSink,
) -> Result<(), PatchError> {
    for &(addr, val) in execution.patch_bytes {
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 1)
            .ok_or(PatchError::BadRange { addr })?;
        rt.place_bytes(AddressSpaceId::BOOT, range, &[val])
            .map_err(|e| PatchError::Refused {
                addr,
                value: val,
                detail: format!("{e:?}"),
            })?;
        if diagnostics.print_banner {
            sink.note(&format!("patch: byte 0x{addr:x} = 0x{val:02x}"));
        }
    }
    Ok(())
}

/// Hex-dump 32 bytes at each `--dump-mem` address, after
/// `module_start` so the dump observes the memory the title sees.
pub(super) fn dump_boot_memory(rt: &Runtime, addrs: &[u64], sink: &dyn crate::BootSink) {
    for &addr in addrs {
        match cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 32) {
            None => sink.note(&format!("mem[0x{addr:x}]: invalid address range")),
            Some(r) => match rt.memory().read(r) {
                Some(slice) => {
                    let label = rt
                        .memory()
                        .containing_region(addr, 32)
                        .map(|r| r.label())
                        .unwrap_or("<unmapped>");
                    let mut line = format!("mem[0x{addr:x}] ({label}):");
                    for b in slice {
                        line.push_str(&format!(" {b:02x}"));
                    }
                    sink.note(&line);
                }
                None => sink.note(&format!("mem[0x{addr:x}]: unmapped")),
            },
        }
    }
}

/// Every module the loop was given is accounted for as started or
/// faulted.
///
/// A faulted start is a witnessed skip (`BENCH_MODULE_START_FAULTS`),
/// so it counts toward completeness. The invariant binds only when the
/// loop ran; the `skip_module_start` boot override runs no module.
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
        .expect("invariant: a static address range");
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

/// Apply the RSX settings `title` opts into: a writable RSX region
/// and the FIFO consumer. Both are off unless the manifest turns them
/// on.
pub(super) fn apply_rsx_settings(rt: &mut Runtime, title: &TitleManifest) {
    if title.rsx_mirror() {
        rt.set_rsx_mirror_writes(true);
    }
    if title.rsx_consume() {
        rt.set_rsx_consume_fifo(true);
    }
}

#[cfg(test)]
#[path = "tests/finish_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/rsx_settings_tests.rs"]
mod rsx_settings_tests;
