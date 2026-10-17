//! The RSX context's driver-info and reports fill, and the RSX free and
//! allocate refusals, held to the `lv2_rsx_answers` capture from the
//! reference console.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::rsx::{driver_info, driver_info_init, reports};

use super::test_helpers::{context_allocate_request, extract_write_u64};
use super::{write_rsx_driver_info_init, write_rsx_reports_init, RSX_CONTEXT_ID};
use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;

/// Payload offsets, as the probe's header names them.
const LOCAL_SIZE: usize = 0x00C;
const MEMORY_FREQ: usize = 0x014;
const CORE_FREQ: usize = 0x018;
const CONTROL_REGISTER: usize = 0x01C;
const LABEL0: usize = 0x020;
const REPORT0: usize = 0x028;
const DRIVER_GUESS: usize = 0x030;
const DRIVER_READABLE: usize = 0x034;
const MEMORY_FREE: usize = 0x040;
const CONTEXT_FREE: usize = 0x048;
const ALLOCATE_ZERO: usize = 0x050;
const ALLOCATE_HANDLE: usize = 0x058;
const ALLOCATE_ADDR: usize = 0x060;
const IOMAP_VALID: usize = 0x068;
const FLIP: usize = 0x080;
const LABELS: usize = 0x100;
const DRIVER: usize = 0x1100;

fn u32_at(payload: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(payload[at..at + 4].try_into().expect("4 bytes"))
}

fn u64_at(payload: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(payload[at..at + 8].try_into().expect("8 bytes"))
}

fn payload() -> Vec<u8> {
    let payload = console_payload("lv2_rsx_answers");
    assert_eq!(payload.len(), DRIVER + driver_info::SIZE);
    assert_eq!(u32_at(&payload, 0), 0, "rsxInit succeeded on the console");
    payload
}

/// The driver-info words the kernel stamps at context allocation, and
/// that CellGov models, agree word for word with the block the console
/// handed libgcm.
#[test]
fn the_driver_info_block_carries_the_console_words() {
    let payload = payload();
    assert_eq!(
        u32_at(&payload, DRIVER_READABLE),
        0,
        "the guess was readable"
    );
    let console = &payload[DRIVER..DRIVER + driver_info::SIZE];
    let mut cellgov = vec![0u8; driver_info::SIZE];
    write_rsx_driver_info_init(&mut cellgov, driver_info_init::MEMORY_SIZE, 0, 1);
    for offset in [
        driver_info::VERSION_DRIVER_OFFSET,
        driver_info::VERSION_GPU_OFFSET,
        driver_info::MEMORY_SIZE_OFFSET,
        driver_info::HARDWARE_CHANNEL_OFFSET,
        driver_info::NVCORE_FREQUENCY_OFFSET,
        driver_info::MEMORY_FREQUENCY_OFFSET,
        driver_info::REPORTS_NOTIFY_OFFSET_FIELD,
        driver_info::REPORTS_OFFSET_FIELD,
        driver_info::REPORTS_REPORT_OFFSET_FIELD,
        driver_info::SYSTEM_MODE_OFFSET,
    ] {
        assert_eq!(
            u32_at(&cellgov, offset),
            u32_at(console, offset),
            "driver_info + {offset:#x}"
        );
    }
    // The handler queue is a kernel id whose value differs between the
    // two; both put one at the same offset.
    assert_ne!(u32_at(console, driver_info::HANDLER_QUEUE_OFFSET), 0);
    // libgcm reads its configuration from the same words.
    assert_eq!(u32_at(&payload, LOCAL_SIZE), driver_info_init::MEMORY_SIZE);
    assert_eq!(
        u32_at(&payload, MEMORY_FREQ),
        driver_info_init::MEMORY_FREQUENCY
    );
    assert_eq!(
        u32_at(&payload, CORE_FREQ),
        driver_info_init::NVCORE_FREQUENCY
    );
}

/// The driver-info block sits one megabyte below the reports block, and
/// the first report entry sits at the published report offset.
#[test]
fn the_reports_block_sits_where_the_driver_info_says() {
    let payload = payload();
    let label0 = u32_at(&payload, LABEL0);
    assert_eq!(u32_at(&payload, DRIVER_GUESS), label0 - 0x10_0000);
    assert_eq!(
        u32_at(&payload, REPORT0),
        label0 + driver_info_init::REPORTS_REPORT_OFFSET
    );
}

/// The semaphore block holds the kernel's fill everywhere libgcm has
/// not set a label. libgcm's bring-up writes the first word of labels
/// 0 to 2.
#[test]
fn the_semaphore_block_carries_the_console_fill() {
    let payload = payload();
    let console = &payload[LABELS..LABELS + driver_info_init::REPORTS_NOTIFY_OFFSET as usize];
    let mut cellgov = vec![0u8; reports::SIZE];
    write_rsx_reports_init(&mut cellgov);
    let mut compared = 0;
    for at in (0..console.len()).step_by(4) {
        if [0x00, 0x10, 0x20].contains(&at) {
            continue;
        }
        assert_eq!(
            u32_at(&cellgov, at),
            u32_at(console, at),
            "semaphore + {at:#x}"
        );
        compared += 1;
    }
    assert_eq!(compared, 1021);
}

#[test]
fn the_free_and_allocate_refusals_answer_as_the_console_does() {
    let payload = payload();
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1_0000);
    let source = UnitId::new(0);
    let immediate = |d: Lv2Dispatch| match d {
        Lv2Dispatch::Immediate { code, effects } => (code, effects),
        other => panic!("expected an immediate answer, got {other:?}"),
    };
    let (_, _) = immediate(host.dispatch(
        Lv2Request::SysRsxMemoryAllocate {
            mem_handle_ptr: 0x1000,
            mem_addr_ptr: 0x1008,
            size: 0x10_0000,
            flags: 0,
            a5: 0,
            a6: 0,
            a7: 0,
        },
        source,
        &rt,
    ));
    let _ = host.dispatch(
        context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 1),
        source,
        &rt,
    );

    let (code, _) = immediate(host.dispatch(
        Lv2Request::SysRsxMemoryFree { mem_handle: 0xDEAD },
        source,
        &rt,
    ));
    assert_eq!(code, u64_at(&payload, MEMORY_FREE), "669(0xDEAD)");
    let (code, _) = immediate(host.dispatch(
        Lv2Request::SysRsxContextFree { context_id: 0xDEAD },
        source,
        &rt,
    ));
    assert_eq!(code, u64_at(&payload, CONTEXT_FREE), "671(0xDEAD)");

    let (code, effects) = immediate(host.dispatch(
        Lv2Request::SysRsxMemoryAllocate {
            mem_handle_ptr: 0x2000,
            mem_addr_ptr: 0x2008,
            size: 0,
            flags: 0,
            a5: 0,
            a6: 0,
            a7: 0,
        },
        source,
        &rt,
    ));
    assert_eq!(code, u64_at(&payload, ALLOCATE_ZERO), "668 with size 0");
    assert_eq!(
        u32_at(&payload, ALLOCATE_HANDLE),
        0xCCCC_CCCC,
        "console wrote no handle"
    );
    assert_eq!(
        u64_at(&payload, ALLOCATE_ADDR),
        0xCCCC_CCCC_CCCC_CCCC,
        "nor an address"
    );
    assert!(effects.is_empty(), "CellGov writes neither");
}

/// The context window: the DMA control block, driver info and reports
/// one megabyte apart, at the addresses the console hands libgcm once
/// the probe's 1 MiB-page host area holds the lowest granule.
#[test]
fn the_context_window_sits_where_the_console_opens_it() {
    let payload = payload();
    let control = u32_at(&payload, CONTROL_REGISTER)
        - cellgov_ps3_abi::lv2::rsx::control_register::PUT_OFFSET;
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
    assert!(matches!(host_area, Lv2Dispatch::Immediate { code: 0, .. }));
    let d = host.dispatch(
        context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 1),
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = d else {
        panic!("expected Immediate(0), got {d:?}");
    };
    assert_eq!(
        extract_write_u64(&effects[1]),
        u64::from(control),
        "dma control"
    );
    assert_eq!(
        extract_write_u64(&effects[2]),
        u64::from(u32_at(&payload, DRIVER_GUESS)),
        "driver info"
    );
    assert_eq!(
        extract_write_u64(&effects[3]),
        u64::from(u32_at(&payload, LABEL0)),
        "reports"
    );
}

/// A game's own raw iomap and flip calls get `CELL_EPERM` from the
/// console, even with a mapping libgcm would make. CellGov answers the
/// context id it hands out, so these paths stay unprobed from a game.
#[test]
fn the_console_refuses_a_game_s_raw_iomap_and_flip_that_cellgov_answers() {
    let payload = payload();
    let eperm: u64 = errno::CELL_EPERM.into();
    assert_eq!(u64_at(&payload, IOMAP_VALID), eperm, "the valid iomap");
    assert_eq!(u64_at(&payload, FLIP), eperm, "the flip");
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1_0000);
    let _ = host.dispatch(
        context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 1),
        UnitId::new(0),
        &rt,
    );
    let d = host.dispatch(
        Lv2Request::SysRsxContextIomap {
            context_id: RSX_CONTEXT_ID,
            io: 0x10_0000,
            ea: 0x5010_0000,
            size: 0x10_0000,
            flags: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(
        matches!(d, Lv2Dispatch::Immediate { code: 0, ref effects } if effects.is_empty()),
        "{d:?}"
    );
}

/// The console refuses `sys_rsx_attribute` with a first argument of 0 or
/// `0xDEAD`; CellGov answers every call CELL_OK, because the packages
/// firmware sends are unprobed.
#[test]
fn the_console_refuses_rsx_attribute_shapes_cellgov_answers() {
    let payload = console_payload("lv2_rsx_attribute");
    assert_eq!(payload.len(), 24);
    assert_eq!(u64_at(&payload, 0), 0, "rsxInit succeeded on the console");
    let einval: u64 = errno::CELL_EINVAL.into();
    for (k, first) in [0u64, 0xDEAD].into_iter().enumerate() {
        assert_eq!(u64_at(&payload, 8 * (k + 1)), einval, "console, {first:#x}");
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let mut args = [0u64; 8];
        args[0] = first;
        assert_eq!(
            host.dispatch(
                Lv2Request::Unsupported { number: 677, args },
                UnitId::new(0),
                &rt
            ),
            Lv2Dispatch::immediate(0),
            "CellGov, {first:#x}"
        );
    }
}
