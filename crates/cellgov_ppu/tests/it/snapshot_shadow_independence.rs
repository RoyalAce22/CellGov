//! Two `Runtime::snapshot` branches whose shadows are clones of one
//! shadow:
//! - a refresh in one clone never reaches the other;
//! - an invalidation in one clone makes the other miss, so the other
//!   decodes its own memory.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use cellgov_ppu::decode::decode;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_ADDI_R3_R3_1, PPC_BLR};

const SHADOW_BASE: u64 = 0x1000;

fn shadow_bytes() -> [u8; 12] {
    let mut bytes = [0u8; 12];
    bytes[0..4].copy_from_slice(&PPC_ADDI_R3_R3_1.to_be_bytes());
    bytes[4..8].copy_from_slice(&PPC_ADDI_R3_R3_1.to_be_bytes());
    bytes[8..12].copy_from_slice(&PPC_BLR.to_be_bytes());
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

#[test]
fn encoding_decodes_to_expected() {
    match decode(PPC_ADDI_R3_R3_1) {
        Ok(PpuInstruction::Addi { rt, ra, imm }) => {
            assert_eq!(
                (rt, ra, imm),
                (3, 3, 1),
                "PPC_ADDI_R3_R3_1 field encoding drifted"
            );
        }
        Ok(other) => panic!("PPC_ADDI_R3_R3_1 decoded to {other:?}, expected Addi"),
        Err(e) => panic!("PPC_ADDI_R3_R3_1 failed to decode: {e:?}"),
    }
    match decode(PPC_BLR) {
        Ok(PpuInstruction::Bclr { bo, bi, link }) => {
            assert_eq!(
                (bo, bi, link),
                (20, 0, false),
                "PPC_BLR field encoding drifted -- expected unconditional bo=20, bi=0, link=false",
            );
        }
        Ok(other) => panic!("PPC_BLR decoded to {other:?}, expected Bclr"),
        Err(e) => panic!("PPC_BLR failed to decode: {e:?}"),
    }
}

#[test]
fn a_rewrite_in_the_original_never_runs_in_the_clone() {
    let (mut original, mut clone) = build_shadow_pair();

    original.invalidate_range(SHADOW_BASE, 4);
    let rewritten = original.refresh(SHADOW_BASE, PPC_BLR).flatten();
    assert!(
        rewritten.is_some() && original.get(SHADOW_BASE) == rewritten,
        "test setup: the original must run its refreshed word",
    );

    assert_eq!(
        clone.get(SHADOW_BASE),
        None,
        "the clone read a decode from the original's branch",
    );
    clone.refresh(SHADOW_BASE, PPC_ADDI_R3_R3_1);
    assert_eq!(
        clone.get(SHADOW_BASE),
        Some(PpuInstruction::Addi {
            rt: 3,
            ra: 3,
            imm: 1
        }),
        "the clone must run the word of its own memory",
    );
}

#[test]
fn a_rewrite_in_the_clone_never_runs_in_the_original() {
    let (original, mut clone) = build_shadow_pair();

    clone.invalidate_range(SHADOW_BASE, 4);
    let rewritten = clone.refresh(SHADOW_BASE, PPC_BLR).flatten();
    assert!(
        rewritten.is_some() && clone.get(SHADOW_BASE) == rewritten,
        "test setup: the clone must run its refreshed word",
    );

    assert_eq!(
        original.get(SHADOW_BASE),
        None,
        "the original read a decode from the clone's branch",
    );
    let untouched = original.get(SHADOW_BASE + 8);
    assert!(
        untouched.is_some() && untouched == clone.get(SHADOW_BASE + 8),
        "a slot neither branch wrote must keep the shared decode",
    );
}
