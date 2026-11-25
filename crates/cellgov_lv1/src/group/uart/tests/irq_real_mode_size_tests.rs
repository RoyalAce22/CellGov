use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::uart::{GAME_OS_VIRTUAL_UART_IRQ_CALLS, VIRTUAL_UART_IRQ_HANDLERS};
use cellgov_time::GuestTicks;

/// A bitmap address in the real mode region, at the start of a 4 KB page.
const BITMAP: u64 = 0x0001_0000;

/// A thread 0 call with the bitmap [`BITMAP`], at the `sc` of the 4.93
/// kernel's call, whose outlet id a source gives.
fn request() -> HypercallRequest {
    let sc = GAME_OS_VIRTUAL_UART_IRQ_CALLS
        .iter()
        .find(|call| call.firmware == "4.93")
        .map(|call| call.sc)
        .expect("a 4.93 kernel call");
    let mut inputs = [0; 8];
    inputs[0] = BITMAP;
    HypercallRequest {
        unit: UnitId::new(1),
        number: CONFIGURE_VIRTUAL_UART_IRQ,
        inputs,
        pc: sc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(26_389),
    }
}

/// The 4.93 answer to `r`, with `real_mode_size` as the source of the real mode size.
fn configure_from(
    r: &HypercallRequest,
    real_mode_size: fn(&str) -> Option<u64>,
) -> HypercallDisposition {
    configure_irq_with(
        r,
        Some("4.93"),
        &IrqState {
            uart: &UartState::default(),
            ports: &EventReceivePortState::default(),
            storage: &GuestMemory::new(0x2_0000),
        },
        real_mode_size,
    )
}

/// While this holds, the virtual UART interrupt handler does not reach
/// its refusal of a firmware whose real mode size no source gives.
#[test]
fn every_firmware_with_a_virtual_uart_irq_handler_row_has_a_real_mode_size() {
    let firmwares: Vec<&str> = VIRTUAL_UART_IRQ_HANDLERS
        .iter()
        .map(|row| row.firmware)
        .collect();
    assert!(
        !firmwares.is_empty(),
        "no virtual UART interrupt handler row"
    );
    let unsourced: Vec<&str> = firmwares
        .into_iter()
        .filter(|fw| memory::real_mode_size(fw).is_none())
        .collect();
    assert_eq!(unsourced, Vec::<&str>::new());
}

#[test]
fn a_real_mode_bitmap_gets_r3_0_with_the_repository_real_mode_size() {
    let disposition = configure_from(&request(), memory::real_mode_size);
    assert!(
        matches!(&disposition, HypercallDisposition::Answered(answer) if answer.r3 == 0),
        "{disposition:?}"
    );
    // The shipped entry point passes the repository source.
    assert_eq!(
        configure_irq(
            &request(),
            Some("4.93"),
            &IrqState {
                uart: &UartState::default(),
                ports: &EventReceivePortState::default(),
                storage: &GuestMemory::new(0x2_0000),
            },
        ),
        disposition
    );
}

#[test]
fn a_real_mode_bitmap_is_refused_when_no_source_gives_the_real_mode_size() {
    let r = request();
    assert_eq!(
        configure_from(&r, |_: &str| None),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: CONFIGURE_VIRTUAL_UART_IRQ,
            unit: r.unit,
            pc: r.pc,
            what: "a virtual UART bitmap at a firmware whose real mode size no source gives",
        })
    );
}
