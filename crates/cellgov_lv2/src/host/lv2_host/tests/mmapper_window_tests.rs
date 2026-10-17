//! Where a game process's 256 MiB areas and its RSX context window
//! land, held to the `lv2_mmapper_rsx_window` and `lv2_rsx_window_held`
//! captures.

use super::*;
use crate::host::rsx::test_helpers::context_allocate_request;
use crate::host::test_support::{console_payload, FakeRuntime};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;

const AREA: u32 = 0x1000_0000;
const BEFORE_BASES: usize = 2;
const BEFORE_RCS: usize = 10;
const AFTER_BASES: usize = 18;
const TRIES: usize = 8;

fn words() -> Vec<u64> {
    let payload = console_payload("lv2_mmapper_rsx_window");
    assert_eq!(payload.len(), 34 * 8);
    payload
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

/// Every base `mmapper_alloc` grants until it refuses.
fn grants(host: &mut Lv2Host) -> Vec<u64> {
    let mut bases = Vec::new();
    while let Some(base) = host.mmapper_alloc(BOOT_PROCESS_PID, AREA, 0x10_0000) {
        bases.push(u64::from(base));
        assert!(bases.len() <= TRIES, "more grants than the probe tried");
    }
    bases
}

/// Before rsxInit every granule is granted, the one a window would
/// open in included.
#[test]
fn without_a_context_the_grants_are_the_console_s() {
    let w = words();
    let console: Vec<u64> = w[BEFORE_BASES..BEFORE_BASES + TRIES]
        .iter()
        .copied()
        .take_while(|&b| b != 0)
        .collect();
    assert_eq!(
        w[BEFORE_RCS + console.len()],
        u64::from(errno::CELL_ENOMEM),
        "the console refuses the next one"
    );
    assert_eq!(grants(&mut Lv2Host::new()), console);
}

/// After rsxInit the console skips the window's granule, and also the
/// lowest one, which the 1 MiB-page host area the probe hands rsxInit
/// holds.
#[test]
fn with_a_context_the_window_s_granule_is_skipped_as_on_the_console() {
    let w = words();
    assert_eq!(w[0], 0, "rsxInit succeeded on the console");
    let console: Vec<u64> = w[AFTER_BASES..AFTER_BASES + TRIES]
        .iter()
        .copied()
        .take_while(|&b| b != 0)
        .collect();
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1_0000);
    let host_area = host.dispatch(
        crate::request::Lv2Request::MemoryAllocate {
            size: 0x10_0000,
            flags: cellgov_ps3_abi::lv2::memory::page_size::FLAG_1M,
            alloc_addr_ptr: 0x2000,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(
        host_area,
        crate::dispatch::Lv2Dispatch::Immediate { code: 0, .. }
    ));
    let _ = host.dispatch(
        context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 1),
        UnitId::new(0),
        &rt,
    );
    assert_eq!(grants(&mut host), console);
}

/// The window opens in the lowest granule the process holds nothing
/// in: with two areas and a 1 MiB-page host area holding the three
/// lowest, the console opens it in the fourth, and later grants skip
/// it.
#[test]
fn the_window_opens_in_the_lowest_free_granule_as_on_the_console() {
    let w: Vec<u64> = console_payload("lv2_rsx_window_held")
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w.len(), 24);
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1_0000);
    for k in 0..2 {
        assert_eq!(w[2 * k], 0, "held area {k} granted on the console");
        assert_eq!(
            host.mmapper_alloc(BOOT_PROCESS_PID, AREA, 0x10_0000)
                .map(u64::from),
            Some(w[2 * k + 1]),
            "held area {k}"
        );
    }
    let host_area = host.dispatch(
        crate::request::Lv2Request::MemoryAllocate {
            size: 0x10_0000,
            flags: cellgov_ps3_abi::lv2::memory::page_size::FLAG_1M,
            alloc_addr_ptr: 0x2000,
        },
        UnitId::new(0),
        &rt,
    );
    let crate::dispatch::Lv2Dispatch::Immediate { code: 0, effects } = host_area else {
        panic!("expected Immediate(0), got {host_area:?}");
    };
    assert_eq!(
        u64::from(crate::host::test_support::extract_write_u32(&effects[0])),
        w[5],
        "the host area"
    );
    assert_eq!(w[6], 0, "rsxInit succeeded on the console");
    let _ = host.dispatch(
        context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 1),
        UnitId::new(0),
        &rt,
    );
    let put = host
        .sys_rsx_context()
        .control_slot(cellgov_ps3_abi::lv2::rsx::control_register::PUT_OFFSET);
    assert_eq!(put.map(u64::from), Some(w[7]), "libgcm's control register");
    let console: Vec<u64> = w[8..16].iter().copied().take_while(|&b| b != 0).collect();
    assert_eq!(
        w[16 + console.len()],
        u64::from(errno::CELL_ENOMEM),
        "the console refuses the next one"
    );
    assert_eq!(grants(&mut host), console);
}
