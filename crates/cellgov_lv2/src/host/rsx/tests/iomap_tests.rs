//! `sys_rsx_context_iomap` dispatch tests: mapping state recording, context-id keying, and alignment/size rejection.

use super::*;
use crate::host::rsx::test_helpers::context_allocate_request;
use crate::host::test_support::FakeRuntime;
use crate::request::Lv2Request;
use cellgov_event::UnitId;

fn allocate_context(host: &mut Lv2Host) {
    let rt = FakeRuntime::new(0x1_0000);
    let _ = host.dispatch(
        context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 0xA001),
        UnitId::new(0),
        &rt,
    );
}

fn iomap(host: &mut Lv2Host, context_id: u32, io: u32, ea: u32, size: u32) -> Lv2Dispatch {
    let rt = FakeRuntime::new(0x1_0000);
    host.dispatch(
        Lv2Request::SysRsxContextIomap {
            context_id,
            io,
            ea,
            size,
            flags: 0,
        },
        UnitId::new(0),
        &rt,
    )
}

#[test]
fn valid_call_records_mapping_and_returns_ok() {
    let mut host = Lv2Host::new();
    allocate_context(&mut host);
    let d = iomap(&mut host, iomap::CONTEXT_ID, 0, 0x0010_0000, 0x0010_0000);
    let Lv2Dispatch::Immediate { code, effects } = d else {
        panic!("expected Immediate, got {d:?}");
    };
    assert_eq!(code, u64::from(cell_errors::CELL_OK));
    assert!(effects.is_empty(), "iomap is purely state-recording");
    let ctx = host.sys_rsx_context();
    assert_eq!(ctx.iomap_io, 0);
    assert_eq!(ctx.iomap_ea, 0x0010_0000);
    assert_eq!(ctx.iomap_size, 0x0010_0000);
}

#[test]
fn nonzero_io_records_offset() {
    let mut host = Lv2Host::new();
    allocate_context(&mut host);
    let d = iomap(
        &mut host,
        iomap::CONTEXT_ID,
        0x0010_0000,
        0x0020_0000,
        0x0010_0000,
    );
    assert_eq!(d, Lv2Dispatch::immediate(cell_errors::CELL_OK.into()));
    assert_eq!(host.sys_rsx_context().iomap_io, 0x0010_0000);
}

#[test]
fn wrong_context_id_returns_einval() {
    let mut host = Lv2Host::new();
    allocate_context(&mut host);
    let d = iomap(&mut host, 0xDEAD_BEEF, 0, 0x0010_0000, 0x0010_0000);
    assert_eq!(d, Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()));
}

#[test]
fn iomap_before_allocate_keys_only_on_context_id() {
    // RPCS3's sys_rsx.cpp does not gate on host-side
    // context-allocate state; CONTEXT_ID is the only handshake.
    let mut host = Lv2Host::new();
    let d = iomap(&mut host, iomap::CONTEXT_ID, 0, 0x0010_0000, 0x0010_0000);
    assert_eq!(d, Lv2Dispatch::immediate(cell_errors::CELL_OK.into()));
}

#[test]
fn zero_size_returns_einval() {
    let mut host = Lv2Host::new();
    allocate_context(&mut host);
    let d = iomap(&mut host, iomap::CONTEXT_ID, 0, 0x0010_0000, 0);
    assert_eq!(d, Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()));
}

#[test]
fn misaligned_io_ea_or_size_returns_einval() {
    let mut host = Lv2Host::new();
    allocate_context(&mut host);
    for (io, ea, size, label) in [
        (1, 0x0010_0000, 0x0010_0000, "io"),
        (0, 0x0010_0001, 0x0010_0000, "ea"),
        (0, 0x0010_0000, 0x0000_1000, "size"),
    ] {
        let d = iomap(&mut host, iomap::CONTEXT_ID, io, ea, size);
        assert_eq!(
            d,
            Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()),
            "misaligned {label} must reject",
        );
    }
}

#[test]
fn io_plus_size_overflow_returns_einval_and_logs() {
    // This io+size wraps to 0 in u32; any u32 comparison would
    // silently accept it.
    let mut host = Lv2Host::new();
    allocate_context(&mut host);
    let before = host.invariant_break_count();
    let d = iomap(&mut host, iomap::CONTEXT_ID, 0xFFF0_0000, 0, 0x0010_0000);
    assert_eq!(d, Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()));
    assert_eq!(host.invariant_break_count() - before, 1);
}

#[test]
fn io_plus_size_at_exact_cap_is_ok() {
    let mut host = Lv2Host::new();
    allocate_context(&mut host);
    let cap = u32::try_from(PS3_RSX_IOMAP_SIZE).unwrap();
    let d = iomap(
        &mut host,
        iomap::CONTEXT_ID,
        cap - 0x0010_0000,
        0,
        0x0010_0000,
    );
    assert_eq!(d, Lv2Dispatch::immediate(cell_errors::CELL_OK.into()));
}

#[test]
fn oversized_size_returns_einval_and_logs_invariant_break() {
    let mut host = Lv2Host::new();
    allocate_context(&mut host);
    let breaks_before = host.invariant_break_count();
    let too_big = u32::try_from(PS3_RSX_IOMAP_SIZE).unwrap() + 0x0010_0000;
    let d = iomap(&mut host, iomap::CONTEXT_ID, 0, 0x0010_0000, too_big);
    assert_eq!(d, Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()));
    assert_eq!(host.invariant_break_count() - breaks_before, 1);
}

#[test]
fn ea_plus_size_exceeds_local_mem_returns_einval() {
    // Asymmetry: ea-range rejection is plain EINVAL with no
    // invariant break, matching RPCS3's undifferentiated gate.
    let mut host = Lv2Host::new();
    allocate_context(&mut host);
    let before = host.invariant_break_count();
    let local_mem_base = u32::try_from(PS3_RSX_BASE).unwrap();
    let d = iomap(&mut host, iomap::CONTEXT_ID, 0, local_mem_base, 0x0010_0000);
    assert_eq!(d, Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()));
    assert_eq!(host.invariant_break_count(), before);
}

#[test]
fn second_iomap_overwrites_first() {
    let mut host = Lv2Host::new();
    allocate_context(&mut host);
    let _ = iomap(&mut host, iomap::CONTEXT_ID, 0, 0x0010_0000, 0x0010_0000);
    let _ = iomap(
        &mut host,
        iomap::CONTEXT_ID,
        0x0010_0000,
        0x0020_0000,
        0x0010_0000,
    );
    let ctx = host.sys_rsx_context();
    assert_eq!((ctx.iomap_io, ctx.iomap_ea), (0x0010_0000, 0x0020_0000));
}

// WipEout's first iomap call asks for 0x0550_0000 bytes.
const _: () = assert!(PS3_RSX_IOMAP_SIZE >= 0x0550_0000);
