//! `configure_virtual_uart_irq` after a port's shadow bit is set: lv1
//! copies the bit to the bitmap and gives the new outlet an event.

use super::*;
use crate::VirtualUartShadowWrite;
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's call
/// [OFW-4.93 lv2_kernel a:0x8000000000017b0c].
const KERNEL_SC: u64 = 0x8000_0000_0001_7b0c;

/// The kernel's bitmap, in the real mode region: +0x28 of its virtual
/// UART record, which the capture `lv2_vuart_irq_outlet` places at
/// 0x800000000037dc40.
const KERNEL_BITMAP: u64 = 0x0037_dc68;

/// Port 10 is bit 63 - 10 of shadow doubleword 0
/// [OFW-4.93 lv1 a:0x2F2214-0x2F2248].
const PORT_10_BIT: u64 = 1 << 53;

/// lv1 copies its shadow to the bitmap [OFW-4.93 lv1 a:0x2F15FC-0x2F1620],
/// and gives the outlet to the event function while the count of set
/// shadow bits is above 0 [OFW-4.93 lv1 a:0x2F16F4-0x2F1708]. The new
/// outlet has no plug, so the event only marks it pending.
#[test]
fn a_configuration_after_a_raised_port_bit_copies_the_bit_and_gives_the_outlet_an_event() {
    let mut uart = UartState::default();
    uart.apply(UartChange::ShadowSet(VirtualUartShadowWrite {
        index: 0,
        value: PORT_10_BIT,
    }));
    let r = HypercallRequest {
        unit: UnitId::new(1),
        number: CONFIGURE_VIRTUAL_UART_IRQ,
        inputs: [KERNEL_BITMAP, 0, 0, 0, 0, 0, 0, 0],
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    };
    let outlet = game_os_virtual_uart_outlet("4.93", KERNEL_SC).expect("the kernel's outlet");
    let mut bitmap = vec![0; 32];
    bitmap[..8].copy_from_slice(&PORT_10_BIT.to_be_bytes());
    let mut expected = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [outlet]);
    expected.effects.push(Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(KERNEL_BITMAP), 32).expect("the bitmap"),
        bytes: WritePayload::new(bitmap),
        source: r.unit,
        source_time: r.tick,
    });
    expected.push_change(InterruptChange::OutletConstructed(EventReceivePort::new(
        outlet,
    )));
    expected.push_change(InterruptChange::EventPending(outlet));
    expected.push_change(UartChange::IrqConfigured(VirtualUartIrq {
        outlet,
        bitmap: KERNEL_BITMAP,
    }));
    let storage = GuestMemory::new(0x0038_0000);
    let state = IrqState {
        uart: &uart,
        ports: &EventReceivePortState::default(),
        storage: &storage,
    };
    assert_eq!(
        configure_irq(&r, Some("4.93"), &state),
        HypercallDisposition::Answered(expected)
    );
}
