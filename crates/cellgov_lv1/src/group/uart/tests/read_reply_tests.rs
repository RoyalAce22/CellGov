use super::*;
use crate::group::interrupt::InterruptGroupState;
use crate::group::uart::VirtualUartPortParams;
use cellgov_event::UnitId;
use cellgov_mem::{PageSize, Region};
use cellgov_ps3_abi::lv1::uart::GAME_OS_VIRTUAL_UART_PORTS;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's read
/// [OFW-4.93 lv2_kernel a:0x8000000000016b7c].
const KERNEL_READ_SC: u64 = 0x8000_0000_0001_6b7c;

/// The dispatcher manager's port [OFW-4.93 lv1 a:0x5B7E7C-0x5B7E80].
const PORT: u64 = 10;

/// The caller's buffer, in the real mode region.
const BUFFER: u64 = 0x0010_0000;

/// The size of the receive buffer of [`PORT`] at the kernel's open.
fn rx_buffer_size() -> u64 {
    GAME_OS_VIRTUAL_UART_PORTS
        .iter()
        .find(|row| row.firmware == "4.93" && row.port == PORT)
        .expect("a row for the port")
        .rx_buffer_size
}

fn request(address: u64, size: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[PORT, address, size]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: READ_VIRTUAL_UART,
        inputs,
        pc: KERNEL_READ_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(26_803),
    }
}

/// Real mode storage of `size` bytes.
fn storage(size: usize) -> GuestMemory {
    GuestMemory::from_regions(vec![Region::new(0, size, "real mode", PageSize::Page64K)])
        .expect("one region")
}

/// The state with `held` in the receive buffer of [`PORT`] and the mask
/// `mask`, with the triggers at the buffer size less 1.
fn holding(held: &[u8], mask: u64) -> UartState {
    let mut uart = UartState::default();
    uart.apply(UartChange::PortParamsSet(VirtualUartPortParams {
        port: PORT,
        tx_trigger: rx_buffer_size() - 1,
        rx_trigger: rx_buffer_size() - 1,
        interrupt_mask: mask,
    }));
    uart.apply(UartChange::ReceiveBufferSet(ReceivedBytes {
        port: PORT,
        bytes: held.to_vec(),
    }));
    uart
}

fn read_with(
    r: &HypercallRequest,
    uart: &UartState,
    storage: &GuestMemory,
) -> HypercallDisposition {
    read_port(
        r,
        &ReadInput {
            firmware: Some("4.93"),
            memory: &MemoryState::default(),
            uart,
            interrupt: &InterruptGroupState::default(),
            storage,
        },
    )
}

fn taken(r: &HypercallRequest, bytes: &[u8], rest: &[u8]) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [bytes.len() as u64, 0, 0, 0, 0, 0, 0]);
    answer.effects.push(Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(r.r(4)), bytes.len() as u64).expect("a range"),
        bytes: WritePayload::new(bytes.to_vec()),
        source: r.unit,
        source_time: r.tick,
    });
    answer.push_change(UartChange::ReceiveBufferSet(ReceivedBytes {
        port: PORT,
        bytes: rest.to_vec(),
    }));
    HypercallDisposition::Answered(answer)
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: READ_VIRTUAL_UART,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

const HELD: [u8; 6] = [1, 2, 3, 4, 5, 6];

#[test]
fn a_read_of_fewer_bytes_than_the_buffer_holds_takes_the_first_and_keeps_the_rest() {
    let r = request(BUFFER, 4);
    assert_eq!(
        read_with(&r, &holding(&HELD, 0), &storage(0x80_0000)),
        taken(&r, &HELD[..4], &HELD[4..])
    );
}

#[test]
fn a_read_of_more_bytes_than_the_buffer_holds_takes_each_byte() {
    let r = request(BUFFER, 0x100);
    assert_eq!(
        read_with(&r, &holding(&HELD, 0), &storage(0x80_0000)),
        taken(&r, &HELD, &[])
    );
}

#[test]
fn a_read_of_0_bytes_reads_nothing_and_keeps_the_buffer() {
    let r = request(BUFFER, 0);
    assert_eq!(
        read_with(&r, &holding(&HELD, 0), &storage(0x80_0000)),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, [0; 7]))
    );
}

#[test]
fn a_read_whose_buffer_the_storage_does_not_hold_is_a_refusal() {
    let r = request(BUFFER, 4);
    assert_eq!(
        read_with(&r, &holding(&HELD, 0), &storage(0x1000)),
        refused(
            &r,
            "a virtual UART read buffer whose bytes CellGov's storage does not hold"
        )
    );
}

/// lv1 computes the port's status after it copies a byte
/// [OFW-4.93 lv1 a:0x2F0830-0x2F0940]. With the receive trigger at the
/// size less 1, the status has the receive bit while a byte stays. The
/// reply that filled the buffer raised port 10's shadow bit, bit 63 - 10
/// of doubleword 0 [OFW-4.93 lv1 a:0x2F2214-0x2F2248]. The read that takes
/// the last byte clears it [OFW-4.93 lv1 a:0x2F2308-0x2F2334].
#[test]
fn under_mask_2_a_read_that_leaves_bytes_keeps_the_shadow_bit_and_a_read_of_each_byte_clears_it() {
    let mut uart = holding(&HELD, 2);
    uart.apply(UartChange::ShadowSet(crate::VirtualUartShadowWrite {
        index: 0,
        value: 1 << 53,
    }));
    let part = request(BUFFER, 4);
    assert_eq!(
        read_with(&part, &uart, &storage(0x80_0000)),
        taken(&part, &HELD[..4], &HELD[4..])
    );
    let whole = request(BUFFER, 6);
    let HypercallDisposition::Answered(mut cleared) = taken(&whole, &HELD, &[]) else {
        panic!("an answer");
    };
    cleared.push_change(UartChange::ShadowSet(crate::VirtualUartShadowWrite {
        index: 0,
        value: 0,
    }));
    assert_eq!(
        read_with(&whole, &uart, &storage(0x80_0000)),
        HypercallDisposition::Answered(cleared)
    );
}

/// Applies each virtual UART change of `disposition` to `uart`.
fn apply_answer(uart: &mut UartState, disposition: &HypercallDisposition) {
    let HypercallDisposition::Answered(answer) = disposition else {
        panic!("{disposition:?} is no answer");
    };
    for change in &answer.changes {
        if let crate::group::state::StateChange::Uart(change) = change {
            uart.apply((**change).clone());
        }
    }
}

#[test]
fn a_second_read_takes_the_bytes_that_the_first_read_left_in_order() {
    let mut uart = holding(&HELD, 0);
    let first = request(BUFFER, 4);
    let answer = read_with(&first, &uart, &storage(0x80_0000));
    assert_eq!(answer, taken(&first, &HELD[..4], &HELD[4..]));
    apply_answer(&mut uart, &answer);
    assert_eq!(uart.received(PORT), &HELD[4..]);
    let second = request(BUFFER + 0x10, 4);
    let answer = read_with(&second, &uart, &storage(0x80_0000));
    assert_eq!(answer, taken(&second, &HELD[4..], &[]));
    apply_answer(&mut uart, &answer);
    assert_eq!(uart, holding(&[], 0));
}

#[test]
fn a_read_of_an_empty_buffer_reads_0_bytes() {
    let r = request(BUFFER, 4);
    assert_eq!(
        read_with(&r, &holding(&[], 2), &storage(0x80_0000)),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, [0; 7]))
    );
}
