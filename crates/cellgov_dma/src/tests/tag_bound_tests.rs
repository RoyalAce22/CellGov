//! A request cannot carry a tag id the completion cannot publish.
//!
//! The tag-status word is 32 bits, one per tag group, so a completion
//! publishes `1 << tag_id`. A tagged request carries the bound with it,
//! so the completion path shifts without a guard of its own.

use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::hw::spu::{MfcTagId, MFC_MAX_TAG_ID};

use crate::{DmaDirection, DmaRequest};

const LEN: u64 = 4;

fn range(addr: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), LEN).expect("a 4-byte range")
}

fn request() -> DmaRequest {
    DmaRequest::new(DmaDirection::Put, range(0), range(0x100), UnitId::new(1))
        .expect("equal-length ends")
}

#[test]
fn the_highest_architected_tag_id_is_accepted() {
    let tag = MfcTagId::new(MFC_MAX_TAG_ID as u8).expect("31 is in range");
    assert_eq!(tag.status_bit(), 0x8000_0000, "31 is the top bit");
    assert_eq!(
        request().with_tag_id(tag).tag_id(),
        Some(tag),
        "a request carries the tag it was given",
    );
}

#[test]
fn a_tag_id_past_the_architected_range_is_refused() {
    assert_eq!(
        MfcTagId::new(MFC_MAX_TAG_ID as u8 + 1),
        None,
        "32 has no bit in a 32-bit status word",
    );
    assert_eq!(MfcTagId::new(u8::MAX), None, "nor does any wider value");
}

/// Every accepted tag id publishes a distinct bit, and no shift
/// overflows.
///
/// This is the property the type exists for: the completion path shifts
/// by `raw()` without a guard of its own, so the whole accepted range
/// has to be shiftable.
#[test]
fn every_accepted_tag_id_has_its_own_status_bit() {
    let mut seen = 0u32;
    for raw in 0..=u8::MAX {
        let Some(tag) = MfcTagId::new(raw) else {
            continue;
        };
        let bit = tag.status_bit();
        assert_ne!(bit, 0, "tag {raw} publishes no bit");
        assert_eq!(seen & bit, 0, "tag {raw} reuses another tag's bit");
        seen |= bit;
    }
    assert_eq!(
        seen,
        u32::MAX,
        "the accepted range covers the whole status word",
    );
}

/// Tag group `n` publishes the bit of weight two to the `n`, which the
/// distinctness check above leaves open: any permutation of the 32 bits
/// passes it.
// [CBEA p:126 s:9.3.4 MFC Read Tag-Group Query Mask Channel] the tag-group bit table runs g1F down to g0, so group 0 sits in the least significant bit and group 31 in the most significant.
#[test]
fn tag_group_n_publishes_the_bit_of_weight_two_to_the_n() {
    for (raw, bit) in [
        (0u8, 0x0000_0001u32),
        (1, 0x0000_0002),
        (5, 0x0000_0020),
        (30, 0x4000_0000),
        (31, 0x8000_0000),
    ] {
        assert_eq!(
            MfcTagId::new(raw).expect("in range").status_bit(),
            bit,
            "tag {raw}",
        );
    }
}

/// An untagged request is still the PPU and host shape.
#[test]
fn a_request_carries_no_tag_by_default() {
    assert_eq!(
        request().tag_id(),
        None,
        "a PPU or host-initiated transfer names no tag group",
    );
}
