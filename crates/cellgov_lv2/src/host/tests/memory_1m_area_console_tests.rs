//! Where 1 MiB-page `sys_memory_allocate` blocks land and how they
//! share the area range with `sys_mmapper_allocate_address`, held to
//! the `lv2_memory_1m_area` capture.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::memory::page_size;

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, extract_write_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;

const MIB: u64 = 0x10_0000;
const AREA: u64 = 0x1000_0000;

/// A probe's `count` records, as `(r3, address)`.
fn records_of(probe: &str, count: usize) -> Vec<(u64, u64)> {
    let payload = console_payload(probe);
    assert_eq!(payload.len(), count * 16);
    payload
        .chunks_exact(16)
        .map(|r| {
            (
                u64::from_be_bytes(r[..8].try_into().expect("r3")),
                u64::from_be_bytes(r[8..].try_into().expect("address")),
            )
        })
        .collect()
}

fn answer(d: Lv2Dispatch) -> (u64, u64) {
    match d {
        Lv2Dispatch::Immediate { code, effects } => (
            code,
            effects
                .first()
                .map_or(0, |e| u64::from(extract_write_u32(e))),
        ),
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn alloc(host: &mut Lv2Host, rt: &FakeRuntime, size: u64, flags: u64) -> (u64, u64) {
    answer(host.dispatch(
        Lv2Request::MemoryAllocate {
            size,
            flags,
            alloc_addr_ptr: 0x100,
        },
        UnitId::new(0),
        rt,
    ))
}

fn area(host: &mut Lv2Host, rt: &FakeRuntime) -> (u64, u64) {
    answer(host.dispatch(
        Lv2Request::Unsupported {
            number: 330,
            args: [AREA, page_size::FLAG_1M, AREA, 0x100, 0, 0, 0, 0],
        },
        UnitId::new(0),
        rt,
    ))
}

fn free(host: &mut Lv2Host, rt: &FakeRuntime, addr: u64) -> u64 {
    answer(host.dispatch(
        Lv2Request::MemoryFree { addr: addr as u32 },
        UnitId::new(0),
        rt,
    ))
    .0
}

/// Every 1 MiB-page block and area the probe asked for lands where the
/// console put it, the area staying held after its blocks are freed.
/// The 64 KiB-page block (record 5) is placed by CellGov's own user
/// region and is not compared.
#[test]
fn the_1m_page_area_answers_as_the_console_does() {
    let console = records_of("lv2_memory_1m_area", 16);
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    let a = alloc(&mut host, &rt, MIB, page_size::FLAG_1M);
    assert_eq!(a, console[0], "A");
    let b = alloc(&mut host, &rt, MIB, page_size::FLAG_1M);
    assert_eq!(b, console[1], "B");
    let c = alloc(&mut host, &rt, 2 * MIB, page_size::FLAG_1M);
    assert_eq!(c, console[2], "C");
    assert_eq!(free(&mut host, &rt, b.1), console[3].0, "free B");
    let d = alloc(&mut host, &rt, MIB, page_size::FLAG_1M);
    assert_eq!(d, console[4], "D reuses B's base");
    let e = alloc(&mut host, &rt, 0x1_0000, page_size::FLAG_64K);
    assert_eq!(e.0, console[5].0, "E");
    // The console grants the RSX context window's granule before any
    // context opens; CellGov maps the window at boot and skips it, so
    // each area CellGov grants sits one granule past the console's.
    let window = u64::from(cellgov_ps3_abi::lv2::rsx::region::WINDOW_BASE) & !(AREA - 1);
    let past_window =
        |(rc, base): (u64, u64)| (rc, if base >= window { base + AREA } else { base });
    let m1 = area(&mut host, &rt);
    assert_eq!(
        console[6].1, window,
        "the console's M1 is the window's granule"
    );
    assert_eq!(m1, past_window(console[6]), "M1 skips the 1 MiB-page area");
    for (k, block) in [a, c, d].into_iter().enumerate() {
        assert_eq!(free(&mut host, &rt, block.1), console[7 + k].0, "free {k}");
    }
    let m2 = area(&mut host, &rt);
    assert_eq!(
        m2,
        past_window(console[10]),
        "M2: the emptied area stays held"
    );
    let f = alloc(&mut host, &rt, MIB, page_size::FLAG_1M);
    assert_eq!(f, console[11], "F lands at the area's base again");
}

/// A block whose flags name no page size takes 1 MiB pages and lands in
/// the 1 MiB-page area, beside a block that names 1 MiB pages.
#[test]
fn a_block_naming_no_page_size_lands_in_the_1m_page_area() {
    let console = records_of("lv2_memory_default_page", 4);
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    let n = alloc(&mut host, &rt, MIB, 0);
    assert_eq!(n, console[0], "N, flags 0");
    let p = alloc(&mut host, &rt, MIB, page_size::FLAG_1M);
    assert_eq!(p, console[1], "P beside N");
    assert_eq!(free(&mut host, &rt, n.1), console[2].0, "free N");
    assert_eq!(free(&mut host, &rt, p.1), console[3].0, "free P");
}
