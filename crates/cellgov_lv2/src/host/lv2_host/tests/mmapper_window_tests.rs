//! Where a game process's 256 MiB areas land before and after the RSX
//! context window opens, held to the `lv2_mmapper_rsx_window` capture.

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

/// Before rsxInit the console grants every granule, the window's
/// included. CellGov maps the window at boot and never grants its
/// granule, so its grants are the console's less that one.
#[test]
fn without_a_context_the_console_grants_the_window_s_granule_where_cellgov_does_not() {
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
    let window = u64::from(cellgov_ps3_abi::lv2::rsx::region::WINDOW_BASE) & !u64::from(AREA - 1);
    assert!(console.contains(&window), "the console grants it");
    let without_window: Vec<u64> = console.into_iter().filter(|&b| b != window).collect();
    assert_eq!(grants(&mut Lv2Host::new()), without_window);
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

/// A 1 MiB-page area never takes the window's granule, context or not:
/// boot maps the window there. With 330 already holding the lowest
/// granule, as liblv2's start-up reservation does, the area goes past
/// the window.
#[test]
fn a_1m_page_area_skips_the_window_s_granule_before_any_context() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1_0000);
    assert_eq!(
        host.mmapper_alloc(BOOT_PROCESS_PID, AREA, 0x10_0000),
        Some(Lv2Host::MMAPPER_REGION_START)
    );
    let d = host.dispatch(
        crate::request::Lv2Request::MemoryAllocate {
            size: 0x10_0000,
            flags: cellgov_ps3_abi::lv2::memory::page_size::FLAG_1M,
            alloc_addr_ptr: 0x2000,
        },
        UnitId::new(0),
        &rt,
    );
    let crate::dispatch::Lv2Dispatch::Immediate { code: 0, effects } = d else {
        panic!("expected Immediate(0), got {d:?}");
    };
    assert_eq!(
        crate::host::test_support::extract_write_u32(&effects[0]),
        0x7000_0000
    );
}
