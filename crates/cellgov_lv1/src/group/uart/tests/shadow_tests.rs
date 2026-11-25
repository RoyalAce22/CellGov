use super::*;
use crate::answer::UnmodelledError;
use crate::group::interrupt::bitmap::{
    IrqStateBitmapKey, IrqStateBitmapRegistration, StatusShadowWrite,
};
use crate::group::interrupt::event_port::EventReceivePort;
use crate::group::interrupt::{InterruptChange, InterruptGroupState};
use crate::group::uart::VirtualUartIrq;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::WRITE_VIRTUAL_UART;
use cellgov_ps3_abi::lv1::uart::GAME_OS_VIRTUAL_UART_PORTS;
use cellgov_time::GuestTicks;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
const PORT: u64 = 10;

/// Port 10 is bit 63 - 10 of shadow doubleword 0
/// [OFW-4.93 lv1 a:0x2F2214-0x2F2248].
const PORT_BIT: u64 = 1 << 53;

/// The outlet id that lv1 gave the 4.93 kernel's call
/// (`GAME_OS_VIRTUAL_UART_OUTLETS`).
const OUTLET: u64 = 18;

/// A 32-byte UART bitmap and a thread bitmap in the real mode region.
const UART_BITMAP: u64 = 0x0036_1000;
const THREAD_0_BITMAP: u64 = 0x0036_9000;

/// Plug 0 is the most significant bit of status doubleword 0
/// [OFW-4.93 lv1 a:0x2C3794-0x2C37BC].
const PLUG_BIT: u64 = 1 << 63;

/// The size of the receive buffer of [`PORT`] at the kernel's open.
fn size() -> u64 {
    GAME_OS_VIRTUAL_UART_PORTS
        .iter()
        .find(|row| row.firmware == "4.93" && row.port == PORT)
        .expect("a row for the port")
        .rx_buffer_size
}

/// Mask 2 with the receive trigger of the kernel's open, the buffer size
/// less 1 [OFW-4.93 lv2_kernel a:0x80000000000177bc-0x80000000000177ec].
fn params() -> VirtualUartPortParams {
    VirtualUartPortParams {
        port: PORT,
        tx_trigger: size() - 1,
        rx_trigger: size() - 1,
        interrupt_mask: 2,
    }
}

fn thread_0() -> IrqStateBitmapKey {
    IrqStateBitmapKey {
        ppe_id: 1,
        thread: 0,
    }
}

/// Thread 0's bitmap, and the UART's outlet on plug 0 of thread 0.
fn interrupt() -> InterruptGroupState {
    let mut state = InterruptGroupState::default();
    state.apply(InterruptChange::BitmapRegistered(
        IrqStateBitmapRegistration {
            key: thread_0(),
            lpar_address: THREAD_0_BITMAP,
        },
    ));
    state.apply(InterruptChange::OutletConstructed(EventReceivePort::new(
        OUTLET,
    )));
    state.apply(InterruptChange::OutletPlugConnect(EventReceivePort {
        id: OUTLET,
        connection: Some(thread_0()),
        plug: 0,
    }));
    state
}

fn uart(configured: bool, shadow: u64) -> UartState {
    let mut uart = UartState::default();
    if configured {
        uart.apply(UartChange::IrqConfigured(VirtualUartIrq {
            outlet: OUTLET,
            bitmap: UART_BITMAP,
        }));
    }
    uart.apply(UartChange::ShadowSet(VirtualUartShadowWrite {
        index: 0,
        value: shadow,
    }));
    uart
}

fn request() -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: WRITE_VIRTUAL_UART,
        inputs: [PORT, 0, 0, 0, 0, 0, 0, 0],
        pc: 0x8000_0000_0001_6fd8,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// The answer that `status_step` builds from an empty answer, with
/// storage that holds both bitmaps.
fn step(uart: &UartState, received: u64) -> Result<HypercallAnswer, UnmodelledError> {
    step_with_storage(uart, received, 0x0037_0000)
}

/// The answer that `status_step` builds from an empty answer, with
/// `storage_bytes` bytes of storage from address 0.
fn step_with_storage(
    uart: &UartState,
    received: u64,
    storage_bytes: usize,
) -> Result<HypercallAnswer, UnmodelledError> {
    let interrupt = interrupt();
    let storage = GuestMemory::new(storage_bytes);
    let input = StatusInput {
        uart,
        raise: RaiseState {
            bitmaps: interrupt.bitmaps(),
            ports: interrupt.ports(),
            paused: interrupt.paused(),
            storage: &storage,
        },
    };
    let mut answer = HypercallAnswer::lv1_status(0, []);
    status_step(&request(), "4.93", &input, &params(), received, &mut answer)?;
    Ok(answer)
}

fn write(address: u64, value: u64) -> Effect {
    Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(address), 8).expect("a doubleword"),
        bytes: WritePayload::new(value.to_be_bytes().to_vec()),
        source: UnitId::new(1),
        source_time: GuestTicks::ZERO,
    }
}

fn shadow_set(value: u64) -> UartChange {
    UartChange::ShadowSet(VirtualUartShadowWrite { index: 0, value })
}

/// The first raised bit gives the outlet an event, and the event raises
/// plug 0 of thread 0, the caller [OFW-4.93 lv1 a:0x2F227C-0x2F2280]
/// [OFW-4.93 lv1 a:0x2DF8BC-0x2DF8C8].
#[test]
fn the_first_raised_port_bit_raises_the_outlet_s_plug() {
    let mut expected = HypercallAnswer::lv1_status(0, []);
    expected.push_change(shadow_set(PORT_BIT));
    expected.effects.push(write(UART_BITMAP, PORT_BIT));
    expected.push_change(InterruptChange::EventPending(OUTLET));
    expected.effects.push(write(THREAD_0_BITMAP, PLUG_BIT));
    expected.push_change(InterruptChange::StatusShadow(StatusShadowWrite {
        key: thread_0(),
        index: 0,
        value: PLUG_BIT,
    }));
    assert_eq!(step(&uart(true, 0), 0x48), Ok(expected));
}

/// lv1 calls the event function only when the count becomes 1
/// [OFW-4.93 lv1 a:0x2F2270]. The count covers each doubleword of the
/// shadow, so a set bit in the last doubleword also stops the event.
#[test]
fn a_raise_beside_another_set_bit_gives_no_event() {
    let mut expected = HypercallAnswer::lv1_status(0, []);
    expected.push_change(shadow_set(PORT_BIT | 1));
    expected.effects.push(write(UART_BITMAP, PORT_BIT | 1));
    assert_eq!(step(&uart(true, 1), 0x48), Ok(expected));

    let mut last_doubleword_set = uart(true, 0);
    last_doubleword_set.apply(UartChange::ShadowSet(VirtualUartShadowWrite {
        index: 3,
        value: 1,
    }));
    let mut expected = HypercallAnswer::lv1_status(0, []);
    expected.push_change(shadow_set(PORT_BIT));
    expected.effects.push(write(UART_BITMAP, PORT_BIT));
    assert_eq!(step(&last_doubleword_set, 0x48), Ok(expected));
}

/// lv1 clears the bit and stores the doubleword
/// [OFW-4.93 lv1 a:0x2F2308-0x2F2334].
#[test]
fn an_empty_receive_buffer_clears_a_set_port_bit() {
    let mut expected = HypercallAnswer::lv1_status(0, []);
    expected.push_change(shadow_set(0));
    expected.effects.push(write(UART_BITMAP, 0));
    assert_eq!(step(&uart(true, PORT_BIT), 0), Ok(expected));
}

/// lv1 changes nothing for a bit that the status gives
/// [OFW-4.93 lv1 a:0x2F2250-0x2F2258].
#[test]
fn a_set_bit_that_the_status_still_shares_stays() {
    assert_eq!(
        step(&uart(true, PORT_BIT), 0x48),
        Ok(HypercallAnswer::lv1_status(0, []))
    );
}

/// Before `configure_virtual_uart_irq`, +48 names the object's own
/// doublewords and +16 holds no outlet [OFW-4.93 lv1 a:0x2F1270]
/// [OFW-4.93 lv1 a:0x2F12AC-0x2F12BC]
/// [OFW-4.93 lv1 a:0x2F2394-0x2F2398].
#[test]
fn a_raise_before_the_irq_configuration_keeps_the_bit_only() {
    let mut expected = HypercallAnswer::lv1_status(0, []);
    expected.push_change(shadow_set(PORT_BIT));
    assert_eq!(step(&uart(false, 0), 0x48), Ok(expected));
}

/// The storage ends at the first byte of the UART bitmap.
#[test]
fn a_raise_into_a_bitmap_that_the_storage_does_not_hold_is_a_refusal() {
    assert_eq!(
        step_with_storage(&uart(true, 0), 0x48, UART_BITMAP as usize),
        Err(UnmodelledError::new(
            "a virtual UART interrupt bitmap that CellGov holds no storage for",
        ))
    );
}

/// A first raised bit from thread 1 gives the outlet an event on plug 0
/// of thread 0, which runs. lv1 signals that thread with an
/// interprocessor interrupt [OFW-4.93 lv1 a:0x2C663C-0x2C6650].
#[test]
fn a_raise_on_the_plug_of_another_running_thread_signals_that_thread() {
    let uart = uart(true, 0);
    let interrupt = interrupt();
    let storage = GuestMemory::new(0x0037_0000);
    let input = StatusInput {
        uart: &uart,
        raise: RaiseState {
            bitmaps: interrupt.bitmaps(),
            ports: interrupt.ports(),
            paused: interrupt.paused(),
            storage: &storage,
        },
    };
    let other = HypercallRequest {
        thread: Some(1),
        ..request()
    };
    let mut answer = HypercallAnswer::lv1_status(0, []);
    assert_eq!(
        status_step(&other, "4.93", &input, &params(), 0x48, &mut answer),
        Ok(())
    );
    let mut expected = HypercallAnswer::lv1_status(0, []);
    expected.push_change(shadow_set(PORT_BIT));
    expected.effects.push(write(UART_BITMAP, PORT_BIT));
    expected.push_change(InterruptChange::EventPending(OUTLET));
    expected.effects.push(write(THREAD_0_BITMAP, PLUG_BIT));
    expected.push_change(InterruptChange::StatusShadow(StatusShadowWrite {
        key: thread_0(),
        index: 0,
        value: PLUG_BIT,
    }));
    expected.push_change(InterruptChange::ThreadSignalled(0));
    assert_eq!(answer, expected);
}
