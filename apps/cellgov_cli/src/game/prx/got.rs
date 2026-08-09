//! GOT-patch staging: bind imports to firmware exports or to
//! unresolved-import trampolines. One staging buffer, applied
//! atomically; guest memory is unchanged on any per-item or batch
//! validation failure.

use std::collections::BTreeMap;

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, StagedWrite, StagingMemory};

use super::types::PrxLoadStageError;

const UNRESOLVED_TRAMP_BODY_BYTES: usize = 24;

/// Encode the unresolved-import trampoline body for `nid`: load NID
/// into r4, set r11 to [`cellgov_ps3_abi::syscall::UNRESOLVED_IMPORT`],
/// `sc`, `blr`.
///
/// The `clrldi r4, r4, 32` clears the upper 32 bits because
/// [PowerISA-3.1 p:I76 s:3.3.9] sign-extends `lis rT, SI` (extended
/// mnemonic for `addis rT, 0, SI`) to 64 bits when `SI` has bit 15
/// set; without the clear, the classifier's u32 narrow rejects half
/// of all NIDs.
fn build_unresolved_trampoline_body(nid: u32) -> [u8; UNRESOLVED_TRAMP_BODY_BYTES] {
    let hi = (nid >> 16) & 0xFFFF;
    let lo = nid & 0xFFFF;
    // lis  r4,  hi          -- 0x3C80_0000 | hi
    let lis_r4 = 0x3C80_0000u32 | hi;
    // ori  r4, r4, lo       -- 0x6084_0000 | lo
    let ori_r4 = 0x6084_0000u32 | lo;
    // clrldi r4, r4, 32     -- rldicl r4, r4, 0, 32 = 0x7884_0020
    let clrldi_r4 = 0x7884_0020u32;
    // lis  r11, 0x0001      -- 0x3D60_0001 (sets r11 = 0x10000)
    let lis_r11 = 0x3D60_0001u32;
    // sc with LEV=0         -- 0x4400_0002
    let sc = 0x4400_0002u32;
    // blr                   -- 0x4E80_0020
    let blr = 0x4E80_0020u32;
    let mut bytes = [0u8; UNRESOLVED_TRAMP_BODY_BYTES];
    bytes[0..4].copy_from_slice(&lis_r4.to_be_bytes());
    bytes[4..8].copy_from_slice(&ori_r4.to_be_bytes());
    bytes[8..12].copy_from_slice(&clrldi_r4.to_be_bytes());
    bytes[12..16].copy_from_slice(&lis_r11.to_be_bytes());
    bytes[16..20].copy_from_slice(&sc.to_be_bytes());
    bytes[20..24].copy_from_slice(&blr.to_be_bytes());
    bytes
}

/// Each trampoline slot: 8-byte OPD + 24-byte body, packed.
const UNRESOLVED_TRAMP_SLOT_BYTES: u32 = 8 + UNRESOLVED_TRAMP_BODY_BYTES as u32;

/// Outcome of an atomic GOT patch batch.
#[derive(Debug, Clone, Default)]
pub(super) struct GotPatchStats {
    /// Imports patched to firmware OPDs.
    pub(super) resolved: usize,
    /// Imports the iteration considered, including unresolved.
    pub(super) total: usize,
    /// Imports patched to the unresolved-import trampoline (no
    /// firmware export covers the NID).
    pub(super) trampolined: usize,
    /// End of the trampoline region (caller advances the allocator
    /// floor here).
    pub(super) tramp_region_end: u64,
    /// Trampolined NID -> the libraries whose import tables asked for
    /// it. Diagnostic payload for the host's unresolved-import arm;
    /// the trampoline itself carries only the NID.
    pub(super) unresolved_requesters: BTreeMap<u32, std::collections::BTreeSet<String>>,
    /// Variable imports the batch saw but did not bind. There is no
    /// variable-import binder yet, so each vref slot keeps its
    /// pre-load bytes; callers report the count so the gap is
    /// witnessed rather than silent.
    pub(super) variables_unbound: usize,
}

/// Stage the OPD + sc-issuing body for one unresolved-import
/// trampoline at `slot_base`. Mirrors the OPD layout
/// `{ code = body_addr, toc = 0 }`; the body size is fixed by
/// [`UNRESOLVED_TRAMP_BODY_BYTES`].
fn stage_unresolved_trampoline(staging: &mut StagingMemory, slot_base: u64, nid: u32) {
    let body_addr = slot_base + 8;

    let mut opd_bytes = [0u8; 8];
    opd_bytes[0..4].copy_from_slice(&(body_addr as u32).to_be_bytes());
    let opd_range = ByteRange::new(GuestAddr::new(slot_base), 8).expect("trampoline OPD range");
    staging.stage(StagedWrite {
        range: opd_range,
        bytes: opd_bytes.to_vec(),
    });

    let body = build_unresolved_trampoline_body(nid);
    let body_range = ByteRange::new(
        GuestAddr::new(body_addr),
        UNRESOLVED_TRAMP_BODY_BYTES as u64,
    )
    .expect("trampoline body range");
    staging.stage(StagedWrite {
        range: body_range,
        bytes: body.to_vec(),
    });
}

/// Stage one 4-byte GOT write per import and apply the whole batch
/// atomically. Unresolved NIDs route to a trampoline allocated from
/// the region starting at `tramp_base`; on any per-item or batch
/// validation failure the staging buffer is dropped and guest memory
/// is unchanged. Callers must advance their alloc floor past
/// [`GotPatchStats::tramp_region_end`].
pub(super) fn patch_got_atomic(
    modules: &[cellgov_ppu::prx::ImportedModule],
    mem: &mut GuestMemory,
    tramp_base: u64,
    mut lookup: impl FnMut(&str, u32) -> Option<u64>,
) -> Result<GotPatchStats, PrxLoadStageError> {
    let mut staging = StagingMemory::new();
    let mut stats = GotPatchStats {
        tramp_region_end: tramp_base,
        ..GotPatchStats::default()
    };
    // Keyed by NID alone even though lookup is namespaced: the
    // trampoline's only payload is the NID, so two libraries with the
    // same unresolved NID would stage byte-identical slots. Sharing
    // one keeps the region size independent of how imports are
    // grouped.
    let mut nid_to_tramp_opd: BTreeMap<u32, u64> = BTreeMap::new();
    let mut next_tramp_offset: u64 = 0;

    // Iterated per module rather than flat_mapped over functions so
    // the library name reaches `lookup`; an import names the library
    // it wants, and resolving without it rebinds to whichever module
    // happens to export the NID.
    for module in modules {
        // No variable-import binder exists yet; count what it would
        // have bound so the gap stays witnessed at the call sites.
        stats.variables_unbound += module.variables.len();
        for func in &module.functions {
            stats.total += 1;

            let opd_u32 = if let Some(addr) = lookup(&module.name, func.nid) {
                stats.resolved += 1;
                u32::try_from(addr).map_err(|_| PrxLoadStageError::GotSlotValueBeyondU32 {
                    nid: func.nid,
                    addr,
                })?
            } else {
                stats.trampolined += 1;
                stats
                    .unresolved_requesters
                    .entry(func.nid)
                    .or_default()
                    .insert(module.name.clone());
                let slot_base = if let Some(&existing) = nid_to_tramp_opd.get(&func.nid) {
                    existing
                } else {
                    let slot_base = tramp_base + next_tramp_offset;
                    // The OPD word and the body pointer inside the
                    // slot are 4-byte guest addresses; reject a slot
                    // whose end does not fit rather than truncate.
                    if slot_base
                        .checked_add(u64::from(UNRESOLVED_TRAMP_SLOT_BYTES))
                        .and_then(|end| u32::try_from(end).ok())
                        .is_none()
                    {
                        return Err(PrxLoadStageError::GotSlotValueBeyondU32 {
                            nid: func.nid,
                            addr: slot_base,
                        });
                    }
                    next_tramp_offset += UNRESOLVED_TRAMP_SLOT_BYTES as u64;
                    stage_unresolved_trampoline(&mut staging, slot_base, func.nid);
                    nid_to_tramp_opd.insert(func.nid, slot_base);
                    slot_base
                };
                slot_base as u32
            };

            let range = ByteRange::new(GuestAddr::new(func.stub_addr as u64), 4).ok_or(
                PrxLoadStageError::GotSlotBadRange {
                    stub_addr: func.stub_addr,
                    nid: func.nid,
                },
            )?;
            staging.stage(StagedWrite {
                range,
                bytes: opd_u32.to_be_bytes().to_vec(),
            });
        }
    }

    stats.tramp_region_end = tramp_base + next_tramp_offset;

    staging
        .drain_into(mem)
        .map_err(|source| PrxLoadStageError::GotBatchCommit {
            staged: stats.resolved + stats.trampolined,
            source,
        })?;
    Ok(stats)
}

#[cfg(test)]
#[path = "tests/got_tests.rs"]
mod tests;
