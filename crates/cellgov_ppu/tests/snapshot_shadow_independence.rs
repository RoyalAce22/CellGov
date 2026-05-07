//! Pins the per-unit predecoded shadow non-aliasing invariant for
//! `Runtime::snapshot`. `PredecodedShadow` carries `slots`, `stale`,
//! and `block_len` `Vec`s that `invalidate_range` mutates in place
//! (SMC, CRT0 relocations, HLE trampoline planting); a switch to
//! `Arc`-shared storage in any of them would let branch A's
//! invalidate corrupt branch B's snapshot.
//!
//! Lives in `cellgov_ppu` because `cellgov_core` cannot depend on
//! `cellgov_ppu` (DAG rule). The end-to-end registry path is
//! mechanical glue and pinned indirectly by the snapshot tests in
//! `cellgov_core::runtime::snapshot`, which catch any `clone_box`
//! aliasing via post-snapshot mutation.

use cellgov_ppu::decode::decode;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::shadow::PredecodedShadow;

/// `addi r3, r3, 1`. Verified by [`encoding_decodes_to_expected`].
const ADDI_R3_R3_1: u32 = 0x3863_0001;

/// `blr`. Verified by [`encoding_decodes_to_expected`].
const BLR: u32 = 0x4E80_0020;

const SHADOW_BASE: u64 = 0x1000;

/// Two `addi` followed by `blr`: yields `block_len = [3, 2, 1]`
/// so the block-length aliasing assertion has signal.
fn shadow_bytes() -> [u8; 12] {
    let mut bytes = [0u8; 12];
    bytes[0..4].copy_from_slice(&ADDI_R3_R3_1.to_be_bytes());
    bytes[4..8].copy_from_slice(&ADDI_R3_R3_1.to_be_bytes());
    bytes[8..12].copy_from_slice(&BLR.to_be_bytes());
    bytes
}

fn build_shadow_pair() -> (PredecodedShadow, PredecodedShadow) {
    let bytes = shadow_bytes();
    let original = PredecodedShadow::build(SHADOW_BASE, &bytes);
    let clone = original.clone();

    let pre_orig = original.get(SHADOW_BASE);
    let pre_clone = clone.get(SHADOW_BASE);
    assert!(
        pre_orig.is_some() && pre_clone.is_some(),
        "test setup: both shadows must decode the slot at construction \
         (orig={pre_orig:?}, clone={pre_clone:?})",
    );
    assert_eq!(
        pre_orig, pre_clone,
        "test setup: original and clone must agree before mutation",
    );

    (original, clone)
}

/// Without this guard, a typo in `ADDI_R3_R3_1` or `BLR` could
/// silently decode to a different valid PPC64 instruction and the
/// aliasing tests would vouch for the wrong invariant.
#[test]
fn encoding_decodes_to_expected() {
    match decode(ADDI_R3_R3_1) {
        Ok(PpuInstruction::Addi { rt, ra, imm }) => {
            assert_eq!(
                (rt, ra, imm),
                (3, 3, 1),
                "ADDI_R3_R3_1 field encoding drifted"
            );
        }
        Ok(other) => panic!("ADDI_R3_R3_1 decoded to {other:?}, expected Addi"),
        Err(e) => panic!("ADDI_R3_R3_1 failed to decode: {e:?}"),
    }
    match decode(BLR) {
        Ok(PpuInstruction::Bclr { bo, bi, link }) => {
            assert_eq!(
                (bo, bi, link),
                (20, 0, false),
                "BLR field encoding drifted -- expected unconditional bo=20, bi=0, link=false",
            );
        }
        Ok(other) => panic!("BLR decoded to {other:?}, expected Bclr"),
        Err(e) => panic!("BLR failed to decode: {e:?}"),
    }
}

#[test]
fn cloned_shadow_unaffected_by_invalidate_on_original() {
    let (mut original, clone) = build_shadow_pair();
    let unaffected_pre = clone.get(SHADOW_BASE);
    let unaffected_pre_block_len = clone.block_len_at(SHADOW_BASE);
    assert!(
        unaffected_pre_block_len > 1,
        "test setup: shadow_bytes must produce a multi-instruction \
         basic block (got block_len_at(base) = {unaffected_pre_block_len})",
    );

    // Mutate the original (simulates SMC / CRT0 reloc on a snapshot's host).
    original.invalidate_range(SHADOW_BASE, 4);

    // Original now reports the slot stale AND its block length collapsed.
    assert!(
        original.get(SHADOW_BASE).is_none(),
        "test setup: invalidate_range must stale the slot in the original",
    );
    assert_eq!(
        original.block_len_at(SHADOW_BASE),
        1,
        "test setup: invalidate_range must collapse block_len in the original",
    );

    // The clone must NOT have been affected. Aliasing canary: if any
    // of slots / stale / block_len ever shares storage via Arc, one
    // of these assertions fires.
    assert_eq!(
        clone.get(SHADOW_BASE),
        unaffected_pre,
        "shadow `slots`/`stale` aliased the original -- \
         branch A invalidate leaked into branch B (via slot lookup)",
    );
    assert_eq!(
        clone.block_len_at(SHADOW_BASE),
        unaffected_pre_block_len,
        "shadow `block_len` aliased the original -- \
         branch A invalidate leaked into branch B (via block-length collapse)",
    );
}

#[test]
fn cloned_shadow_invalidate_does_not_propagate_to_original() {
    let (original, mut clone) = build_shadow_pair();
    let unaffected_pre = original.get(SHADOW_BASE);
    let unaffected_pre_block_len = original.block_len_at(SHADOW_BASE);

    clone.invalidate_range(SHADOW_BASE, 4);

    assert!(
        clone.get(SHADOW_BASE).is_none(),
        "test setup: invalidate_range must stale the slot in the clone",
    );
    assert_eq!(
        clone.block_len_at(SHADOW_BASE),
        1,
        "test setup: invalidate_range must collapse block_len in the clone",
    );

    assert_eq!(
        original.get(SHADOW_BASE),
        unaffected_pre,
        "shadow `slots`/`stale` aliased the clone -- \
         branch B invalidate leaked into branch A (via slot lookup)",
    );
    assert_eq!(
        original.block_len_at(SHADOW_BASE),
        unaffected_pre_block_len,
        "shadow `block_len` aliased the clone -- \
         branch B invalidate leaked into branch A (via block-length collapse)",
    );
}
