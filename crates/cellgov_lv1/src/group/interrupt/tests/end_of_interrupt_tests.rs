use super::*;
use crate::answer::ReturnPath;
use crate::external::returns;
use crate::group::interrupt::bitmap::{IrqStateBitmapRegistration, StatusShadowWrite};
use crate::group::interrupt::event_port::EventReceivePort;
use crate::group::state::GroupStates;
use crate::group::uart::{UartChange, VirtualUartIrq, VirtualUartShadowWrite};
use crate::{
    answered_numbers, ExternalDelivery, ExternalRaise, Hypervisor, RunningThreads, SavedMsrs,
};
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT,
    SEND_EVENT_LOCALLY,
};
use cellgov_ps3_abi::lv1::interrupt::IRQ_MASK_OFFSET;
use cellgov_time::GuestTicks;

/// The kernel's call in its external interrupt dispatch
/// [OFW-4.93 lv2_kernel a:0x8000000000008d9c].
const EOI_SC: u64 = 0x8000_0000_0000_8d9c;

/// The `sc` of the kernel's event send
/// [OFW-4.93 lv2_kernel a:0x8000000000025a34].
const SEND_SC: u64 = 0x8000_0000_0002_5a34;

/// The `sc` of the kernel's port call in the setup of a thread's record
/// [OFW-4.93 lv2_kernel a:0x8000000000007d8c]. The console gave thread 1's
/// call the port id 4 (`GAME_OS_PORT_CALLS`).
const RECORD_SC: u64 = 0x8000_0000_0000_7d8c;

/// The port of thread 1's record setup.
const PORT: u64 = 4;

/// The outlet id that lv1 gave the 4.93 kernel's virtual UART call
/// (`GAME_OS_VIRTUAL_UART_OUTLETS`).
const UART_OUTLET: u64 = 18;

/// An irq outlet id of a logical SPE.
const SPE_OUTLET: u64 = 20;

/// The partition's logical PPE id at 4.93 (`game_os_logical_ppe_id`).
const PPE: u64 = 1;

/// A bitmap address in the real mode region, 8-byte aligned.
const THREAD_1_BITMAP: u64 = 0x0036_9200;

/// A 32-byte UART bitmap in the real mode region.
const UART_BITMAP: u64 = 0x0036_1000;

/// The plug that the tests connect the object to.
const PLUG: u8 = 9;

/// Plug 9 is bit 63 - 9 of status doubleword 0 (`plug_status_bit`).
const PLUG_BIT: u64 = 1 << 54;

/// Another bit of status doubleword 0, of plug 63.
const OTHER_BIT: u64 = 1;

/// The 4.93 kernel's MSR at its calls: SF, EE, IR, DR and RI.
const KERNEL_MSR: u64 = 0x8000_0000_0000_8032;

/// The external interrupt that a return with [`KERNEL_MSR`] raises
/// [OFW-4.93 lv1 a:0x18FC-0x1914].
const DELIVERY: ExternalRaise = ExternalRaise::Deliver(ExternalDelivery {
    vector: 0x500,
    msr: 0x8000_0000_0000_0000,
});

fn thread_1() -> IrqStateBitmapKey {
    IrqStateBitmapKey {
        ppe_id: PPE,
        thread: 1,
    }
}

fn call(number: u64, args: &[u64], pc: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

fn eoi(ppe: u64, thread: u64, plug: u64) -> HypercallRequest {
    call(END_OF_INTERRUPT_EXT, &[ppe, thread, plug], EOI_SC)
}

/// Storage whose thread 1 bitmap holds `mask` in mask doubleword 0.
fn storage(mask: u64) -> GuestMemory {
    let mut storage = GuestMemory::new(0x0037_0000);
    let address = THREAD_1_BITMAP + IRQ_MASK_OFFSET;
    let range = ByteRange::new(GuestAddr::new(address), 8).expect("the mask doubleword");
    storage
        .apply_commit(range, &mask.to_be_bytes())
        .expect("the mask write");
    storage
}

/// Thread 1's bitmap, and the object `id` on [`PLUG`] of thread 1. The
/// object is an outlet when `outlet` is set, and a port otherwise.
/// `pending` sets its pending byte, and `shadow` is status shadow
/// doubleword 0.
fn interrupt(id: u64, outlet: bool, pending: bool, shadow: u64) -> InterruptGroupState {
    let mut state = InterruptGroupState::default();
    state.apply(InterruptChange::BitmapRegistered(
        IrqStateBitmapRegistration {
            key: thread_1(),
            lpar_address: THREAD_1_BITMAP,
        },
    ));
    let connected = EventReceivePort {
        id,
        connection: Some(thread_1()),
        plug: PLUG,
    };
    if outlet {
        state.apply(InterruptChange::OutletConstructed(EventReceivePort::new(
            id,
        )));
        state.apply(InterruptChange::OutletPlugConnect(connected));
    } else {
        state.apply(InterruptChange::PortConstructed(EventReceivePort::new(id)));
        state.apply(InterruptChange::PlugConnect(connected));
    }
    if pending {
        state.apply(InterruptChange::EventPending(id));
    }
    state.apply(InterruptChange::StatusShadow(StatusShadowWrite {
        key: thread_1(),
        index: 0,
        value: shadow,
    }));
    state
}

/// The virtual UART with [`UART_OUTLET`] and `shadow` in its shadow
/// doubleword `index`.
fn uart_at(index: u8, shadow: u64) -> UartState {
    let mut uart = UartState::default();
    uart.apply(UartChange::IrqConfigured(VirtualUartIrq {
        outlet: UART_OUTLET,
        bitmap: UART_BITMAP,
    }));
    uart.apply(UartChange::ShadowSet(VirtualUartShadowWrite {
        index,
        value: shadow,
    }));
    uart
}

/// The virtual UART with [`UART_OUTLET`] and `shadow` in its shadow
/// doubleword 0.
fn uart(shadow: u64) -> UartState {
    uart_at(0, shadow)
}

fn end_at(
    r: &HypercallRequest,
    firmware: Option<&str>,
    interrupt: &InterruptGroupState,
    uart: &UartState,
) -> HypercallDisposition {
    let storage = storage(0);
    end_of_interrupt(
        r,
        firmware,
        &EndState {
            interrupt,
            uart,
            storage: &storage,
        },
    )
}

fn end(
    r: &HypercallRequest,
    firmware: &str,
    interrupt: &InterruptGroupState,
    uart: &UartState,
) -> HypercallDisposition {
    end_at(r, Some(firmware), interrupt, uart)
}

fn answered(disposition: HypercallDisposition) -> HypercallAnswer {
    match disposition {
        HypercallDisposition::Answered(answer) => answer,
        refused => panic!("{refused:?}"),
    }
}

fn write(address: u64, value: u64) -> Effect {
    Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(address), 8).expect("a doubleword"),
        bytes: WritePayload::new(value.to_be_bytes().to_vec()),
        source: UnitId::new(1),
        source_time: GuestTicks::ZERO,
    }
}

fn shadow(value: u64) -> InterruptChange {
    InterruptChange::StatusShadow(StatusShadowWrite {
        key: thread_1(),
        index: 0,
        value,
    })
}

/// lv1's success answer for the object `id`: the cleared pending event,
/// and the write of `cleared` to status doubleword 0 with its shadow.
fn cleared(id: u64, cleared: u64) -> HypercallAnswer {
    let mut answer = HypercallAnswer::lv1_status(0, []);
    answer.push_change(InterruptChange::EventCleared(id));
    answer.effects.push(write(THREAD_1_BITMAP, cleared));
    answer.push_change(shadow(cleared));
    answer
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: END_OF_INTERRUPT_EXT,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

#[test]
fn the_hypervisor_answers_30() {
    assert!(answered_numbers().contains(&END_OF_INTERRUPT_EXT));
}

/// The handler checks r3, then r4, then r5
/// [OFW-4.93 lv1 a:0x2E98CC-0x2E98D8] [OFW-4.93 lv1 a:0x2E99B4-0x2E99C8].
#[test]
fn each_failed_check_gets_the_r3_of_the_first_failure_and_clears_r4_to_r10() {
    let state = interrupt(PORT, false, true, PLUG_BIT);
    let uart = UartState::default();
    let cases = [
        (eoi(2, 1, 9), END_OF_INTERRUPT_PPE_ID_R3),
        (eoi(0, 1, 9), END_OF_INTERRUPT_PPE_ID_R3),
        (eoi(2, 2, 0x100), END_OF_INTERRUPT_PPE_ID_R3),
        (eoi(PPE, 2, 9), END_OF_INTERRUPT_THREAD_R3),
        (eoi(PPE, 1 << 32, 9), END_OF_INTERRUPT_THREAD_R3),
        (eoi(PPE, 2, 0x100), END_OF_INTERRUPT_THREAD_R3),
        (eoi(PPE, 1, 0x100), END_OF_INTERRUPT_PLUG_R3),
        (eoi(PPE, 1, 1 << 40), END_OF_INTERRUPT_PLUG_R3),
    ];
    assert_eq!(cases.len(), 8);
    for (r, r3) in cases {
        assert_eq!(
            end(&r, "4.93", &state, &uart),
            HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [])),
            "{:?}",
            r.inputs
        );
    }
}

/// An empty slot returns 0x96095A04 [OFW-4.93 lv1 a:0x2DFEB0-0x2DFEC0].
#[test]
fn a_plug_that_holds_no_object_gets_wrong_state_with_r5_marked() {
    let state = interrupt(PORT, false, true, PLUG_BIT);
    for r in [eoi(PPE, 1, 10), eoi(PPE, 0, 9)] {
        assert_eq!(
            end(&r, "4.93", &state, &UartState::default()),
            HypercallDisposition::Answered(HypercallAnswer::lv1_status(
                END_OF_INTERRUPT_WRONG_STATE_R3,
                []
            )),
            "{:?}",
            r.inputs
        );
    }
}

/// A pending byte of 0 returns 0x96095A05 before the clear
/// [OFW-4.93 lv1 a:0x2DFCCC-0x2DFCD4].
#[test]
fn a_connected_port_with_no_pending_raise_gets_unmarked_wrong_state_and_no_change() {
    let state = interrupt(PORT, false, false, PLUG_BIT);
    assert_eq!(
        end(&eoi(PPE, 1, 9), "4.93", &state, &UartState::default()),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(
            END_OF_INTERRUPT_NOT_PENDING_R3,
            []
        ))
    );
}

/// [OFW-4.93 lv1 a:0x2DFCF8] [OFW-4.93 lv1 a:0x2C37C0-0x2C37D8]
#[test]
fn a_pending_port_clears_its_pending_byte_and_only_its_plug_bit() {
    let state = interrupt(PORT, false, true, PLUG_BIT | OTHER_BIT);
    assert_eq!(
        end(&eoi(PPE, 1, 9), "4.93", &state, &uart(1)),
        HypercallDisposition::Answered(cleared(PORT, OTHER_BIT))
    );
}

/// The UART's node gives the outlet to the event function while the
/// count of set shadow bits is above 0 [OFW-4.93 lv1 a:0x2F11D0-0x2F11E0].
/// The count covers the whole shadow, so a bit in any doubleword counts.
/// The event sets the pending byte and raises the bit that the clear
/// cleared [OFW-4.93 lv1 a:0x2DF87C] [OFW-4.93 lv1 a:0x2C6620-0x2C6634].
#[test]
fn the_uart_outlet_raises_its_plug_again_while_any_uart_shadow_doubleword_holds_a_bit() {
    let state = interrupt(UART_OUTLET, true, true, PLUG_BIT);
    let mut expected = cleared(UART_OUTLET, 0);
    expected.push_change(InterruptChange::EventPending(UART_OUTLET));
    expected.effects.push(write(THREAD_1_BITMAP, PLUG_BIT));
    expected.push_change(shadow(PLUG_BIT));
    let indices = u8::try_from(UART_SHADOW_DOUBLEWORDS).expect("the UART shadow doublewords");
    assert_eq!(indices, 4);
    for index in 0..indices {
        assert_eq!(
            end(&eoi(PPE, 1, 9), "4.93", &state, &uart_at(index, 1 << 53)),
            HypercallDisposition::Answered(expected.clone()),
            "UART shadow doubleword {index}"
        );
    }
}

#[test]
fn the_uart_outlet_stays_cleared_when_the_uart_shadow_is_0() {
    let state = interrupt(UART_OUTLET, true, true, PLUG_BIT);
    assert_eq!(
        end(&eoi(PPE, 1, 9), "4.93", &state, &uart(0)),
        HypercallDisposition::Answered(cleared(UART_OUTLET, 0))
    );
}

/// The SPE outlet constructor links a node to the outlet's list
/// [OFW-4.93 lv1 a:0x2CB274-0x2CB2A4].
#[test]
fn a_pending_irq_outlet_of_a_logical_spe_is_a_refusal() {
    let state = interrupt(SPE_OUTLET, true, true, PLUG_BIT);
    let r = eoi(PPE, 1, 9);
    assert_eq!(
        end(&r, "4.93", &state, &uart(1 << 53)),
        refused(
            &r,
            "an end of interrupt on an irq outlet of a logical SPE, whose node list lv1 runs"
        )
    );
}

/// The pending byte of 0 returns before the node list runs
/// [OFW-4.93 lv1 a:0x2DFCCC-0x2DFCD4] [OFW-4.93 lv1 a:0x2DFD88-0x2DFDDC].
#[test]
fn an_irq_outlet_of_a_logical_spe_with_no_pending_event_gets_unmarked_wrong_state() {
    let state = interrupt(SPE_OUTLET, true, false, PLUG_BIT);
    assert_eq!(
        end(&eoi(PPE, 1, 9), "4.93", &state, &UartState::default()),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(
            END_OF_INTERRUPT_NOT_PENDING_R3,
            []
        ))
    );
}

/// The plug check refuses only an r5 above 0xFF
/// [OFW-4.93 lv1 a:0x2E99C4-0x2E99C8], so plug 0xFF reaches the slot.
#[test]
fn plug_0xff_passes_the_plug_check_and_reaches_its_empty_slot() {
    let state = interrupt(PORT, false, true, PLUG_BIT);
    assert_eq!(
        end(&eoi(PPE, 1, 0xFF), "4.93", &state, &UartState::default()),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(
            END_OF_INTERRUPT_WRONG_STATE_R3,
            []
        ))
    );
}

#[test]
fn a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let state = interrupt(PORT, false, true, PLUG_BIT);
    let r = eoi(PPE, 1, 9);
    for firmware in [None, Some("4.92"), Some("3.70")] {
        assert_eq!(
            end_at(&r, firmware, &state, &UartState::default()),
            refused(
                &r,
                "an end of interrupt at a firmware whose lv1 handler is not read"
            ),
            "{firmware:?}"
        );
    }
}

/// After the clear, the return ANDs the shadow with the mask again
/// [OFW-4.93 lv1 a:0x0FC4-0x101C]. The UART's raise sets the bit again,
/// so the return raises the external interrupt again.
#[test]
fn the_return_after_a_uart_raise_again_raises_the_external_interrupt() {
    let mut groups = GroupStates::default();
    let state = interrupt(UART_OUTLET, true, true, PLUG_BIT);
    for change in [
        InterruptChange::BitmapRegistered(IrqStateBitmapRegistration {
            key: thread_1(),
            lpar_address: THREAD_1_BITMAP,
        }),
        InterruptChange::OutletConstructed(EventReceivePort::new(UART_OUTLET)),
        InterruptChange::OutletPlugConnect(*state.ports().outlet(UART_OUTLET).expect("outlet")),
        InterruptChange::EventPending(UART_OUTLET),
        shadow(PLUG_BIT),
    ] {
        groups.apply(change.into());
    }
    for change in [
        UartChange::IrqConfigured(VirtualUartIrq {
            outlet: UART_OUTLET,
            bitmap: UART_BITMAP,
        }),
        UartChange::ShadowSet(VirtualUartShadowWrite {
            index: 0,
            value: 1 << 53,
        }),
    ] {
        groups.apply(change.into());
    }
    let storage = storage(PLUG_BIT);
    let r = eoi(PPE, 1, 9);
    let answer = answered(end_of_interrupt(
        &r,
        Some("4.93"),
        &EndState {
            interrupt: groups.interrupt(),
            uart: groups.uart(),
            storage: &storage,
        },
    ));
    assert!(matches!(answer.path, ReturnPath::Table(_)));
    let msrs = SavedMsrs([Some(KERNEL_MSR); 2]);
    let running = RunningThreads::default();
    let returned = answered(returns(
        &r,
        Some("4.93"),
        &groups,
        &storage,
        &msrs,
        &running,
        answer,
    ));
    assert_eq!(returned.external, Some(DELIVERY));
}

/// A 4.93 hypervisor whose thread 1 registered its bitmap, constructed
/// [`PORT`] and connected it to [`PLUG`] of thread 1.
fn booted(storage: &GuestMemory) -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let bitmap = call(
        CONFIGURE_IRQ_STATE_BITMAP,
        &[PPE, 1, THREAD_1_BITMAP],
        SEND_SC,
    );
    answered(hv.dispatch_with(&bitmap, storage));
    let port = call(CONSTRUCT_EVENT_RECEIVE_PORT, &[], RECORD_SC);
    assert_eq!(
        answered(hv.dispatch_with(&port, storage))
            .outputs()
            .map(|o| o[0]),
        Some(PORT)
    );
    let plug = call(
        CONNECT_IRQ_PLUG_EXT,
        &[PPE, 1, u64::from(PLUG), PORT, 0],
        SEND_SC,
    );
    assert_eq!(answered(hv.dispatch_with(&plug, storage)).r3, 0);
    hv
}

fn dispatch(hv: &mut Hypervisor, r: &HypercallRequest, storage: &GuestMemory) -> HypercallAnswer {
    answered(hv.dispatch_returning(r, storage, &SavedMsrs([Some(KERNEL_MSR); 2])))
}

/// The send's return raises the interrupt for the bit that the mask
/// holds. The end of interrupt clears the bit, so its return raises none.
#[test]
fn the_return_after_a_port_s_end_of_interrupt_raises_no_external_interrupt() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(&storage);
    let send = call(SEND_EVENT_LOCALLY, &[PORT], SEND_SC);
    assert_eq!(dispatch(&mut hv, &send, &storage).external, Some(DELIVERY));
    let answer = dispatch(&mut hv, &eoi(PPE, 1, 9), &storage);
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.external, None);
    assert!(!hv.event_receive_ports().is_pending(PORT));
    assert_eq!(hv.irq_state_bitmaps().shadow(thread_1())[0], 0);
}

/// While the pending byte is set, the event function returns early
/// [OFW-4.93 lv1 a:0x2DF864-0x2DF86C]. The end of interrupt clears it.
#[test]
fn the_end_of_interrupt_lets_the_next_event_raise_the_plug_again() {
    let storage = storage(0);
    let mut hv = booted(&storage);
    let send = call(SEND_EVENT_LOCALLY, &[PORT], SEND_SC);
    dispatch(&mut hv, &send, &storage);
    assert!(dispatch(&mut hv, &send, &storage).changes.is_empty());
    dispatch(&mut hv, &eoi(PPE, 1, 9), &storage);
    let again = dispatch(&mut hv, &send, &storage);
    assert_eq!(
        again.changes,
        vec![
            InterruptChange::EventPending(PORT).into(),
            shadow(PLUG_BIT).into()
        ]
    );
}

#[test]
fn an_ended_interrupt_leaves_the_hash_of_the_state_before_the_event() {
    let storage = storage(0);
    let mut hv = booted(&storage);
    let before = hv.sync_term();
    let send = call(SEND_EVENT_LOCALLY, &[PORT], SEND_SC);
    dispatch(&mut hv, &send, &storage);
    assert_ne!(hv.sync_term(), before);
    dispatch(&mut hv, &eoi(PPE, 1, 9), &storage);
    assert_eq!(hv.sync_term(), before);
}
