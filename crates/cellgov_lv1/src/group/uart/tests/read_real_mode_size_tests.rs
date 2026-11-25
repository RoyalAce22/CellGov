use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::uart::{GAME_OS_SERVICE_PORTS, VIRTUAL_UART_READ_HANDLERS};
use cellgov_time::GuestTicks;

/// A buffer address in the real mode region, away from its end.
const BUFFER: u64 = 0x0001_0000;

const REFUSED_WHAT: &str =
    "a virtual UART read buffer at a firmware whose real mode size no source gives";

/// The first 4.93 service port, whose call passes each check before the
/// region checks.
fn service_port() -> u64 {
    GAME_OS_SERVICE_PORTS
        .iter()
        .find(|row| row.firmware == "4.93")
        .map(|row| row.port)
        .expect("a 4.93 service port")
}

/// A thread 0 read of 0 bytes of the service port into [`BUFFER`], at the
/// `sc` of the 4.93 kernel's read [OFW-4.93 lv2_kernel a:0x8000000000016b7c].
fn request() -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[service_port(), BUFFER, 0]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: READ_VIRTUAL_UART,
        inputs,
        pc: 0x8000_0000_0001_6b7c,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(26_548),
    }
}

/// A virtual UART state whose service port receive buffer holds 4 bytes.
fn holding() -> UartState {
    let mut uart = UartState::default();
    uart.apply(UartChange::ReceiveBufferSet(ReceivedBytes {
        port: service_port(),
        bytes: vec![0x5A; 4],
    }));
    uart
}

/// The 4.93 answer to `r` for `uart`, with `real_mode_size` as the source
/// of the real mode size.
fn read_from(
    r: &HypercallRequest,
    uart: &UartState,
    real_mode_size: fn(&str) -> Option<u64>,
) -> HypercallDisposition {
    read_port_with(
        r,
        &ReadInput {
            firmware: Some("4.93"),
            memory: &MemoryState::default(),
            uart,
            interrupt: &InterruptGroupState::default(),
            storage: &GuestMemory::new(0x2_0000),
        },
        real_mode_size,
    )
}

fn refused(r: &HypercallRequest) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: READ_VIRTUAL_UART,
        unit: r.unit,
        pc: r.pc,
        what: REFUSED_WHAT,
    })
}

/// While this holds, the read handler does not reach its refusal of a
/// firmware whose real mode size no source gives.
#[test]
fn every_firmware_with_a_read_handler_row_has_a_real_mode_size() {
    let firmwares: Vec<&str> = VIRTUAL_UART_READ_HANDLERS
        .iter()
        .map(|row| row.firmware)
        .collect();
    assert!(!firmwares.is_empty(), "no read handler row");
    let unsourced: Vec<&str> = firmwares
        .into_iter()
        .filter(|fw| memory::real_mode_size(fw).is_none())
        .collect();
    assert_eq!(unsourced, Vec::<&str>::new());
}

#[test]
fn a_real_mode_read_buffer_passes_the_region_checks_with_the_repository_real_mode_size() {
    let r = request();
    let read_0_bytes = HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, []));
    assert_eq!(
        read_from(&r, &UartState::default(), memory::real_mode_size),
        read_0_bytes
    );
    assert_eq!(
        read_from(&r, &holding(), memory::real_mode_size),
        read_0_bytes
    );
    // The shipped entry point passes the repository source.
    for uart in [UartState::default(), holding()] {
        let input = ReadInput {
            firmware: Some("4.93"),
            memory: &MemoryState::default(),
            uart: &uart,
            interrupt: &InterruptGroupState::default(),
            storage: &GuestMemory::new(0x2_0000),
        };
        assert_eq!(read_port(&r, &input), read_0_bytes);
    }
}

#[test]
fn a_read_of_an_empty_port_is_refused_when_no_source_gives_the_real_mode_size() {
    let r = request();
    assert_eq!(
        read_from(&r, &UartState::default(), |_: &str| None),
        refused(&r)
    );
}

#[test]
fn a_read_of_a_port_with_bytes_is_refused_when_no_source_gives_the_real_mode_size() {
    let r = request();
    assert_eq!(read_from(&r, &holding(), |_: &str| None), refused(&r));
}
