//! The `CLEAR_ALL` wait mode the flag table cannot express.

use super::*;
use crate::host::test_support::{
    extract_write_u32, fake_runtime_with_valid_sync_attr, seed_primary_ppu, VALID_SYNC_ATTR_PTR,
};
use crate::request::Lv2Request;
use cellgov_ps3_abi::lv2::sync::event_flag_wait_mode as wait_mode;

const WAIT_SITE: &str = "event_flag.wait_clear_all_not_modeled";
const TRYWAIT_SITE: &str = "event_flag.trywait_clear_all_not_modeled";

/// Create a flag holding `init` and return its id.
fn flag_with(host: &mut Lv2Host, rt: &impl Lv2Runtime, src: UnitId, init: u64) -> u32 {
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            init,
        },
        src,
        rt,
    );
    match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn clear_all_wait_leaves_bits_outside_the_pattern_and_is_witnessed() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let id = flag_with(&mut host, &rt, src, 0b1111);

    let w = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b0001,
            mode: wait_mode::OR | wait_mode::CLEAR_ALL,
            result_ptr: 0x200,
            timeout: 0,
        },
        src,
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, .. } = w else {
        panic!("expected Immediate(0), got {w:?}");
    };

    // CLEAR_ALL asks for a zeroed flag value. The model clears only the
    // waited-on pattern, so 0b1110 survives -- which is exactly the
    // divergence the witness exists to name.
    assert_eq!(host.event_flags().lookup(id).unwrap().bits(), 0b1110);
    assert_eq!(host.invariant_break_site_count(WAIT_SITE), 1);
}

#[test]
fn clear_mode_clears_the_pattern_without_witnessing_a_collapse() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let id = flag_with(&mut host, &rt, src, 0b1111);

    let w = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b0001,
            mode: wait_mode::OR | wait_mode::CLEAR,
            result_ptr: 0x200,
            timeout: 0,
        },
        src,
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, .. } = w else {
        panic!("expected Immediate(0), got {w:?}");
    };

    assert_eq!(host.event_flags().lookup(id).unwrap().bits(), 0b1110);
    assert_eq!(host.invariant_break_site_count(WAIT_SITE), 0);
}

#[test]
fn clear_all_trywait_is_witnessed_at_its_own_site() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let id = flag_with(&mut host, &rt, src, 0b1111);

    host.dispatch(
        Lv2Request::EventFlagTryWait {
            id,
            bits: 0b0001,
            mode: wait_mode::OR | wait_mode::CLEAR_ALL,
            result_ptr: 0x200,
        },
        src,
        &rt,
    );

    assert_eq!(host.invariant_break_site_count(TRYWAIT_SITE), 1);
    assert_eq!(host.invariant_break_site_count(WAIT_SITE), 0);
}

#[test]
fn a_rejected_mode_is_not_witnessed_as_a_clear_all_collapse() {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let id = flag_with(&mut host, &rt, src, 0b1111);

    // Both clear bits at once is not a legal clear nibble.
    let w = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b0001,
            mode: wait_mode::OR | wait_mode::CLEAR | wait_mode::CLEAR_ALL,
            result_ptr: 0x200,
            timeout: 0,
        },
        src,
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = w else {
        panic!("expected Immediate, got {w:?}");
    };
    assert_eq!(code, errno::CELL_EINVAL.into());
    assert_eq!(host.invariant_break_site_count(WAIT_SITE), 0);
}
