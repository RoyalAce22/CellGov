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
#[derive(Debug, Clone, Copy, Default)]
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
    mut lookup: impl FnMut(u32) -> Option<u64>,
) -> Result<GotPatchStats, PrxLoadStageError> {
    let mut staging = StagingMemory::new();
    let mut stats = GotPatchStats {
        tramp_region_end: tramp_base,
        ..GotPatchStats::default()
    };
    let mut nid_to_tramp_opd: BTreeMap<u32, u64> = BTreeMap::new();
    let mut next_tramp_offset: u64 = 0;

    for func in modules.iter().flat_map(|m| m.functions.iter()) {
        stats.total += 1;

        let opd_u32 = if let Some(addr) = lookup(func.nid) {
            stats.resolved += 1;
            addr as u32
        } else {
            stats.trampolined += 1;
            let slot_base = if let Some(&existing) = nid_to_tramp_opd.get(&func.nid) {
                existing
            } else {
                let slot_base = tramp_base + next_tramp_offset;
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
