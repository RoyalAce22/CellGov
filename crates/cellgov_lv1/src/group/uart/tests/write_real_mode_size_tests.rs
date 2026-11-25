use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::uart::{GAME_OS_SERVICE_PORTS, VIRTUAL_UART_WRITE_HANDLERS};
use cellgov_time::GuestTicks;

/// A buffer address in the real mode region, away from its end.
const BUFFER: u64 = 0x0001_0000;

/// The first 4.93 service port, whose call passes each check before the
/// region checks.
fn service_port() -> u64 {
    GAME_OS_SERVICE_PORTS
        .iter()
        .find(|row| row.firmware == "4.93")
        .map(|row| row.port)
        .expect("a 4.93 service port")
}

/// A thread 0 write of 0 bytes at [`BUFFER`] to the service port, at the
/// `sc` of the 4.93 kernel's write [OFW-4.93 lv2_kernel a:0x8000000000016fd8].
fn request() -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[service_port(), BUFFER, 0]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: WRITE_VIRTUAL_UART,
        inputs,
        pc: 0x8000_0000_0001_6fd8,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(26_776),
    }
}

/// The 4.93 answer to `r`, with `real_mode_size` as the source of the real mode size.
fn write_from(
    r: &HypercallRequest,
    real_mode_size: fn(&str) -> Option<u64>,
) -> HypercallDisposition {
    write_with(
        r,
        &WriteInput {
            firmware: Some("4.93"),
            memory: &MemoryState::default(),
            uart: &UartState::default(),
            interrupt: &InterruptGroupState::default(),
            storage: &GuestMemory::new(0x2_0000),
        },
        real_mode_size,
    )
}

/// While this holds, the write handler does not reach its refusal of a
/// firmware whose real mode size no source gives.
#[test]
fn every_firmware_with_a_write_handler_row_has_a_real_mode_size() {
    let firmwares: Vec<&str> = VIRTUAL_UART_WRITE_HANDLERS
        .iter()
        .map(|row| row.firmware)
        .collect();
    assert!(!firmwares.is_empty(), "no write handler row");
    let unsourced: Vec<&str> = firmwares
        .into_iter()
        .filter(|fw| memory::real_mode_size(fw).is_none())
        .collect();
    assert_eq!(unsourced, Vec::<&str>::new());
}

#[test]
fn a_real_mode_write_buffer_passes_the_region_checks_with_the_repository_real_mode_size() {
    let wrote_0_bytes = HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, [0]));
    assert_eq!(
        write_from(&request(), memory::real_mode_size),
        wrote_0_bytes
    );
    // The shipped entry point passes the repository source.
    assert_eq!(
        write(
            &request(),
            &WriteInput {
                firmware: Some("4.93"),
                memory: &MemoryState::default(),
                uart: &UartState::default(),
                interrupt: &InterruptGroupState::default(),
                storage: &GuestMemory::new(0x2_0000),
            },
        ),
        wrote_0_bytes
    );
}

#[test]
fn a_real_mode_write_buffer_is_refused_when_no_source_gives_the_real_mode_size() {
    let r = request();
    assert_eq!(
        write_from(&r, |_: &str| None),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: WRITE_VIRTUAL_UART,
            unit: r.unit,
            pc: r.pc,
            what: "a virtual UART write buffer at a firmware whose real mode size no source gives",
        })
    );
}
