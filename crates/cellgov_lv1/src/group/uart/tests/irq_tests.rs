use super::*;
use crate::group::interrupt::bitmap::IrqStateBitmapKey;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT};
use cellgov_ps3_abi::lv1::repository::{BOOT_RECORD_4_93, BOOT_RECORD_CAPTURE};
use cellgov_ps3_abi::lv1::uart::{
    CONFIGURE_VIRTUAL_UART_IRQ_PAGE_R3, VIRTUAL_UART_IRQ_OUTLET_CAPTURE,
};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's call
/// [OFW-4.93 lv2_kernel a:0x8000000000017b0c].
const KERNEL_SC: u64 = 0x8000_0000_0001_7b0c;

/// The kernel's bitmap: +0x28 of its virtual UART record, which the
/// capture `lv2_vuart_irq_outlet` places at 0x800000000037dc40.
const KERNEL_BITMAP: u64 = 0x0037_dc68;

/// The bitmap of the kernel's thread 0: +0x1C0 of its record, which the
/// capture `lv2_logical_ppe_record` places at 0x8000000000368f80.
const THREAD_0_BITMAP: u64 = 0x0036_9140;

/// The reads of the frame of [`BOOT_RECORD_CAPTURE`]: three locator words
/// and the 24 words of the boot repository record.
const BOOT_RECORD_READS: usize = 27;

/// The offset of `bi.pu.1.rm_size` in the 4.93 kernel's boot repository
/// record. The README of [`BOOT_RECORD_CAPTURE`] gives the offset.
const BOOT_RECORD_RM_SIZE: u64 = 0x38;

/// The outlet id of the kernel's call: the doubleword `outlet` of the
/// capture [`VIRTUAL_UART_IRQ_OUTLET_CAPTURE`]. Both reads of the
/// doubleword must agree.
fn kernel_outlet() -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(VIRTUAL_UART_IRQ_OUTLET_CAPTURE)
        .join("peek/cech20-cex-493/vuart_outlet.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    let row: Vec<&str> = lines
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.get(1) == Some(&"outlet"))
        .unwrap_or_else(|| panic!("{name}: no row for outlet"));
    assert_eq!(row.len(), 6, "{name}: outlet columns");
    assert_eq!(
        (row[2], row[3], row[4]),
        ("8", "2", "yes"),
        "{name}: outlet"
    );
    u64::from_str_radix(row[5].trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("{name}: outlet: {e}"))
}

/// The 4.93 GameOS partition's real mode size, `bi.pu.1.rm_size`: the
/// doubleword that the capture [`BOOT_RECORD_CAPTURE`] read at
/// [`BOOT_RECORD_RM_SIZE`] of the kernel's boot repository record.
fn rm_size() -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(BOOT_RECORD_CAPTURE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    assert_eq!(frame.len(), 8 + len, "one whole frame");
    assert_eq!(len, BOOT_RECORD_READS * 16, "{BOOT_RECORD_READS} reads");
    let word = |b: &[u8]| u64::from_be_bytes(b.try_into().expect("8 bytes"));
    let hits: Vec<u64> = frame[8..]
        .chunks_exact(16)
        .filter(|pair| word(&pair[..8]) == BOOT_RECORD_4_93 + BOOT_RECORD_RM_SIZE)
        .map(|pair| word(&pair[8..]))
        .collect();
    assert_eq!(hits.len(), 1, "{}: one read of rm_size", path.display());
    hits[0]
}

/// Real storage at address 0 that holds the kernel's bitmaps.
fn storage() -> GuestMemory {
    GuestMemory::new(0x0038_0000)
}

/// A thread 0 call of hypercall `number` at the `sc` address `pc`.
fn call(number: u64, pc: u64, args: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(26_389),
    }
}

/// The kernel's call with the bitmap address `bitmap`.
fn request(bitmap: u64) -> HypercallRequest {
    call(CONFIGURE_VIRTUAL_UART_IRQ, KERNEL_SC, &[bitmap])
}

fn configure_in(
    r: &HypercallRequest,
    uart: &UartState,
    ports: &EventReceivePortState,
) -> HypercallDisposition {
    configure_irq(
        r,
        Some("4.93"),
        &IrqState {
            uart,
            ports,
            storage: &storage(),
        },
    )
}

fn fresh(r: &HypercallRequest) -> HypercallDisposition {
    configure_in(r, &UartState::default(), &EventReceivePortState::default())
}

fn lv1_r3(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: CONFIGURE_VIRTUAL_UART_IRQ,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

/// lv1 stores the outlet id in the return slot of r4 and returns 0
/// [OFW-4.93 lv1 a:0x2F2A6C-0x2F2A8C]. It copies its 32-byte shadow, 0,
/// to the bitmap [OFW-4.93 lv1 a:0x2F15FC-0x2F1620].
fn configured(r: &HypercallRequest) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [kernel_outlet(), 0, 0, 0, 0, 0, 0]);
    answer.effects.push(Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(KERNEL_BITMAP), 32).expect("the bitmap"),
        bytes: WritePayload::new(vec![0; 32]),
        source: r.unit,
        source_time: r.tick,
    });
    answer.push_change(InterruptChange::OutletConstructed(EventReceivePort {
        id: kernel_outlet(),
        connection: None,
        plug: 0,
    }));
    answer.push_change(UartChange::IrqConfigured(VirtualUartIrq {
        outlet: kernel_outlet(),
        bitmap: KERNEL_BITMAP,
    }));
    HypercallDisposition::Answered(answer)
}

#[test]
fn the_4_93_kernel_call_gets_the_captured_outlet_and_zeroes_its_bitmap() {
    let r = request(KERNEL_BITMAP);
    assert_eq!(fresh(&r), configured(&r));
}

#[test]
fn a_second_call_gets_lv1_s_wrong_state_r3() {
    let mut uart = UartState::default();
    uart.apply(UartChange::IrqConfigured(VirtualUartIrq {
        outlet: kernel_outlet(),
        bitmap: KERNEL_BITMAP,
    }));
    assert_eq!(
        configure_in(
            &request(KERNEL_BITMAP),
            &uart,
            &EventReceivePortState::default()
        ),
        lv1_r3(CONFIGURE_VIRTUAL_UART_IRQ_CONFIGURED_R3)
    );
}

/// lv1 checks r3 before it reads the virtual UART's outlet
/// [OFW-4.93 lv1 a:0x2F289C-0x2F2950] [OFW-4.93 lv1 a:0x2F1474-0x2F1480].
#[test]
fn a_second_call_that_fails_a_region_check_gets_that_check_s_r3() {
    let mut uart = UartState::default();
    uart.apply(UartChange::IrqConfigured(VirtualUartIrq {
        outlet: kernel_outlet(),
        bitmap: KERNEL_BITMAP,
    }));
    let ports = EventReceivePortState::default();
    for (address, r3) in [
        (
            VIRTUAL_UART_IRQ_ADDRESS_MAX + 1,
            CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3,
        ),
        (rm_size(), CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3),
        (rm_size() - 1, CONFIGURE_VIRTUAL_UART_IRQ_REGION_R3),
    ] {
        assert_eq!(
            configure_in(&request(address), &uart, &ports),
            lv1_r3(r3),
            "{address:#x}"
        );
    }
}

/// The handler checks the address bound before the region lookup
/// [OFW-4.93 lv1 a:0x2F289C-0x2F28A8], and the real mode region holds
/// the addresses below `rm_size`.
#[test]
fn an_address_in_no_region_gets_lv1_s_no_region_r3() {
    for address in [
        VIRTUAL_UART_IRQ_ADDRESS_MAX + 1,
        u64::MAX,
        rm_size(),
        rm_size() + 0x1000,
    ] {
        assert_eq!(
            fresh(&request(address)),
            lv1_r3(CONFIGURE_VIRTUAL_UART_IRQ_NO_REGION_R3),
            "{address:#x}"
        );
    }
}

/// lv1 refuses a region with 31 bytes or fewer from r3 to its end
/// [OFW-4.93 lv1 a:0x2F2914-0x2F2924].
#[test]
fn a_bitmap_past_the_real_mode_region_s_end_gets_lv1_s_region_r3() {
    for address in [rm_size() - 31, rm_size() - 8, rm_size() - 1] {
        assert_eq!(
            fresh(&request(address)),
            lv1_r3(CONFIGURE_VIRTUAL_UART_IRQ_REGION_R3),
            "{address:#x}"
        );
    }
}

#[test]
fn a_bitmap_across_a_4_kb_page_is_a_refusal_not_lv1_s_page_r3() {
    let what = "a virtual UART bitmap across a 4 KB page, whose region page size no source gives";
    for address in [rm_size() - 32, 0x0037_dfe0, 0x0037_dff8] {
        let r = request(address);
        let answer = fresh(&r);
        assert_eq!(answer, refused(&r, what), "{address:#x}");
        assert_ne!(answer, lv1_r3(CONFIGURE_VIRTUAL_UART_IRQ_PAGE_R3));
    }
}

#[test]
fn a_bitmap_outside_the_real_mode_region_is_a_refusal() {
    let r = request((28 << 42) | 0x1000_0000);
    assert_eq!(
        fresh(&r),
        refused(
            &r,
            "a virtual UART bitmap outside the partition's real mode region"
        )
    );
}

#[test]
fn a_call_at_another_address_is_a_refusal() {
    let r = call(CONFIGURE_VIRTUAL_UART_IRQ, KERNEL_SC + 4, &[KERNEL_BITMAP]);
    assert_eq!(
        fresh(&r),
        refused(
            &r,
            "a virtual UART interrupt call whose outlet id no source gives for its address"
        )
    );
}

#[test]
fn an_outlet_id_that_a_port_or_an_outlet_holds_is_a_refusal() {
    let what = "a virtual UART interrupt call whose outlet id the hypervisor already holds";
    let r = request(KERNEL_BITMAP);
    let mut with_port = EventReceivePortState::default();
    with_port.construct(EventReceivePort::new(kernel_outlet()));
    let mut with_outlet = EventReceivePortState::default();
    with_outlet.construct_outlet(EventReceivePort::new(kernel_outlet()));
    for ports in [with_port, with_outlet] {
        assert_eq!(
            configure_in(&r, &UartState::default(), &ports),
            refused(&r, what)
        );
    }
}

#[test]
fn a_call_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let r = request(KERNEL_BITMAP);
    let what = "a virtual UART interrupt call at a firmware whose lv1 handler is not read";
    for firmware in [None, Some("4.92")] {
        let answer = configure_irq(
            &r,
            firmware,
            &IrqState {
                uart: &UartState::default(),
                ports: &EventReceivePortState::default(),
                storage: &storage(),
            },
        );
        assert_eq!(answer, refused(&r, what), "{firmware:?}");
    }
}

#[test]
fn a_bitmap_with_no_storage_is_a_refusal() {
    let r = request(KERNEL_BITMAP);
    let answer = configure_irq(
        &r,
        Some("4.93"),
        &IrqState {
            uart: &UartState::default(),
            ports: &EventReceivePortState::default(),
            storage: &GuestMemory::new(0x0037_dc70),
        },
    );
    assert_eq!(
        answer,
        refused(
            &r,
            "a virtual UART bitmap that CellGov holds no storage for"
        )
    );
}

/// The kernel gives the outlet to its connect step after r3 = 0
/// [OFW-4.93 lv2_kernel a:0x8000000000017b2c-0x8000000000017b38].
#[test]
fn the_kernel_s_outlet_connects_to_a_plug_of_thread_0() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let thread_0 = IrqStateBitmapKey {
        ppe_id: 1,
        thread: 0,
    };
    let steps = [
        call(CONFIGURE_IRQ_STATE_BITMAP, 0, &[1, 0, THREAD_0_BITMAP]),
        request(KERNEL_BITMAP),
        call(CONNECT_IRQ_PLUG_EXT, 0, &[1, 0, 7, kernel_outlet(), 0]),
    ];
    let mut r3s = Vec::new();
    for step in steps {
        match hv.dispatch_with(&step, &storage()) {
            HypercallDisposition::Answered(answer) => r3s.push(answer.r3),
            other => panic!("hypercall {}: {other:?}", step.number),
        }
    }
    assert_eq!(r3s, [0, 0, 0]);
    assert_eq!(
        hv.groups().interrupt().ports().outlet(kernel_outlet()),
        Some(&EventReceivePort {
            id: kernel_outlet(),
            connection: Some(thread_0),
            plug: 7,
        })
    );
    assert_eq!(
        hv.groups().uart().irq(),
        Some(&VirtualUartIrq {
            outlet: kernel_outlet(),
            bitmap: KERNEL_BITMAP,
        })
    );
}
