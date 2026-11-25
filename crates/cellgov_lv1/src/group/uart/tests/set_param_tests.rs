use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's call for the transmit trigger
/// [OFW-4.93 lv2_kernel a:0x8000000000017678].
const KERNEL_TX_TRIGGER_SC: u64 = 0x8000_0000_0001_7678;

/// The `sc` of the 4.93 kernel's call for the receive trigger
/// [OFW-4.93 lv2_kernel a:0x80000000000176a0].
const KERNEL_RX_TRIGGER_SC: u64 = 0x8000_0000_0001_76a0;

/// The `sc` of the 4.93 kernel's call that sets the mask
/// [OFW-4.93 lv2_kernel a:0x80000000000175d0].
const KERNEL_MASK_SC: u64 = 0x8000_0000_0001_75d0;

/// The doubleword `field` in the committed capture of the 4.93 kernel's
/// port records. Both reads of the doubleword must agree.
fn captured(field: &str) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(cellgov_ps3_abi::lv1::uart::VIRTUAL_UART_PORT_CAPTURE)
        .join("peek/cech20-cex-493/vuart_ports.tsv");
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
        .find(|row| row.get(1) == Some(&field))
        .unwrap_or_else(|| panic!("{name}: no row for {field}"));
    assert_eq!(row.len(), 6, "{name}: {field} columns");
    assert_eq!(
        (row[2], row[3], row[4]),
        ("8", "2", "yes"),
        "{name}: {field}"
    );
    u64::from_str_radix(row[5].trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("{name}: {field}: {e}"))
}

/// The three ports that the 4.93 kernel opened on the console, as
/// (port, receive buffer size, transmit buffer size), read from the
/// capture's port records.
fn kernel_ports() -> [(u64, u64, u64); 3] {
    [0, 1, 2].map(|slot| {
        (
            captured(&format!("record{slot}_port")),
            captured(&format!("record{slot}_rx")),
            captured(&format!("record{slot}_tx")),
        )
    })
}

/// A thread 0 call of hypercall 164 at the `sc` address `pc`.
fn request_at(port: u64, param: u64, value: u64, pc: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[port, param, value]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: SET_VIRTUAL_UART_PARAM,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(26_457),
    }
}

/// A thread 0 call of hypercall 164 at the kernel's `sc` for `param`.
fn request(port: u64, param: u64, value: u64) -> HypercallRequest {
    let pc = match param {
        0 => KERNEL_TX_TRIGGER_SC,
        1 => KERNEL_RX_TRIGGER_SC,
        _ => KERNEL_MASK_SC,
    };
    request_at(port, param, value, pc)
}

fn at_4_93(r: &HypercallRequest, uart: &UartState) -> HypercallDisposition {
    set_param(r, Some("4.93"), uart)
}

/// lv1 returns 0, and the vector's entry zeroes r4 to r10
/// [OFW-4.93 lv1 a:0x2F3DC8-0x2F3DCC] [OFW-4.93 lv1 a:0x0C08].
fn set(params: VirtualUartPortParams) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(UartChange::PortParamsSet(params));
    HypercallDisposition::Answered(answer)
}

fn lv1_r3(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: SET_VIRTUAL_UART_PARAM,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

/// The refusal of a mask whose lowest bit is `bit` on port 0, whose
/// traffic no source gives.
fn mask_refusal(bit: u64) -> &'static str {
    match bit {
        1 => {
            "the transmit bit 1 of a virtual UART interrupt mask on a port whose peer and \
             traffic no source gives"
        }
        2 => {
            "the receive bit 2 of a virtual UART interrupt mask on a port whose peer and traffic \
             no source gives"
        }
        4 => {
            "the disconnect bit 4 of a virtual UART interrupt mask on a port whose peer and \
             traffic no source gives"
        }
        _ => panic!("bit {bit}"),
    }
}

fn params(port: u64, tx: u64, rx: u64, mask: u64) -> VirtualUartPortParams {
    VirtualUartPortParams {
        port,
        tx_trigger: tx,
        rx_trigger: rx,
        interrupt_mask: mask,
    }
}

/// The kernel registered each port, so lv1 answered each 164 call of the
/// open with r3 = 0; see the capture `lv2_vuart_port_params`. The open
/// sends each buffer size less 1, then the mask 0
/// [OFW-4.93 lv2_kernel a:0x80000000000177bc-0x80000000000177ec].
#[test]
fn the_hypervisor_answers_the_kernel_s_open_of_each_port_and_keeps_its_triggers() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let storage = GuestMemory::new(0x1000);
    let cleared = HypercallAnswer::lv1_status(0, [0; 7]).path;
    for (port, rx, tx) in kernel_ports() {
        let calls = [(0, tx - 1), (1, rx - 1), (2, 0)];
        let answers = calls.map(|(param, value)| {
            let answer = hv.dispatch_with(&request(port, param, value), &storage);
            match answer {
                HypercallDisposition::Answered(answer) => (answer.r3, answer.path),
                refused => panic!("port {port} param {param}: {refused:?}"),
            }
        });
        assert_eq!(answers, [(0, cleared); 3], "port {port}");
        assert_eq!(
            hv.groups().uart().port_params(port),
            params(port, tx - 1, rx - 1, 0),
            "port {port}"
        );
    }
}

#[test]
fn each_trigger_call_carries_the_port_s_values_with_that_trigger_set() {
    let mut uart = UartState::default();
    uart.apply(UartChange::PortParamsSet(params(2, 0x7ff, 0x3ff, 0)));
    assert_eq!(
        at_4_93(&request(2, 0, 0x10), &uart),
        set(params(2, 0x10, 0x3ff, 0))
    );
    assert_eq!(
        at_4_93(&request(2, 1, u64::MAX), &uart),
        set(params(2, 0x7ff, u64::MAX, 0))
    );
}

/// The worker checks the port bound before the parameter
/// [OFW-4.93 lv1 a:0x2F3C30] [OFW-4.93 lv1 a:0x2F3CFC-0x2F3D08].
#[test]
fn a_port_above_255_gets_lv1_s_no_port_r3_whatever_the_parameter() {
    let uart = UartState::default();
    for port in [256, 0x1_0000_0000, u64::MAX] {
        for (param, value) in [(0, 1), (2, 8), (3, 0)] {
            assert_eq!(
                at_4_93(&request(port, param, value), &uart),
                lv1_r3(SET_VIRTUAL_UART_PARAM_NO_PORT_R3),
                "port {port:#x} param {param}"
            );
        }
    }
}

/// lv1 checks the parameter before the mask's value
/// [OFW-4.93 lv1 a:0x2F3DF0-0x2F3E04] [OFW-4.93 lv1 a:0x2F3F40-0x2F3F48].
#[test]
fn a_parameter_above_2_of_a_kernel_port_gets_lv1_s_no_param_r3() {
    let uart = UartState::default();
    for (port, ..) in kernel_ports() {
        for param in [3, 7, 0x1_0000_0002, u64::MAX] {
            assert_eq!(
                at_4_93(&request(port, param, 8), &uart),
                lv1_r3(SET_VIRTUAL_UART_PARAM_NO_PARAM_R3),
                "port {port} param {param:#x}"
            );
        }
    }
}

#[test]
fn a_mask_above_7_gets_lv1_s_mask_r3() {
    let uart = UartState::default();
    for mask in [8, 0x10, 0x1_0000_0000, u64::MAX] {
        assert_eq!(
            at_4_93(&request(10, 2, mask), &uart),
            lv1_r3(SET_VIRTUAL_UART_PARAM_MASK_R3),
            "mask {mask:#x}"
        );
    }
}

/// The kernel sets the mask 2 on each port after the open; see the
/// capture `lv2_vuart_port_mask`.
#[test]
fn a_mask_other_than_0_on_port_0_is_a_refusal_that_names_its_lowest_bit() {
    let uart = UartState::default();
    for (mask, bit) in [(1, 1), (2, 2), (3, 1), (4, 4), (5, 1), (6, 2), (7, 1)] {
        let r = request(0, 2, mask);
        assert_eq!(
            at_4_93(&r, &uart),
            refused(&r, mask_refusal(bit)),
            "mask {mask}"
        );
    }
}

/// lv1 computes the status after a trigger call too
/// [OFW-4.93 lv1 a:0x2F3D70-0x2F3DC4].
#[test]
fn a_trigger_call_on_port_0_whose_mask_is_not_0_is_a_refusal() {
    let mut uart = UartState::default();
    uart.apply(UartChange::PortParamsSet(params(0, 0, 0, 2)));
    for param in [0, 1] {
        let r = request(0, param, 0x7ff);
        assert_eq!(
            at_4_93(&r, &uart),
            refused(&r, mask_refusal(2)),
            "param {param}"
        );
    }
}

#[test]
fn a_mask_of_0_clears_the_mask_the_state_holds() {
    let mut uart = UartState::default();
    uart.apply(UartChange::PortParamsSet(params(0, 0x7ff, 0x7ff, 2)));
    assert_eq!(
        at_4_93(&request(0, 2, 0), &uart),
        set(params(0, 0x7ff, 0x7ff, 0))
    );
}

/// lv1 looks the port up before it checks the parameter
/// [OFW-4.93 lv1 a:0x2F3C34-0x2F3D08].
#[test]
fn a_port_that_no_source_gives_is_a_refusal_whatever_the_parameter() {
    let uart = UartState::default();
    let what = "a virtual UART port that no source shows lv1 holds or lacks";
    for port in [1, 3, 11, 255] {
        for (param, value) in [(0, 1), (2, 8), (3, 0)] {
            let r = request(port, param, value);
            assert_eq!(
                at_4_93(&r, &uart),
                refused(&r, what),
                "port {port} param {param}"
            );
        }
    }
}

/// The answer reads no register but r3 to r5, so it is the same at any pc.
#[test]
fn the_answer_is_the_same_at_any_pc() {
    let uart = UartState::default();
    for pc in [0, KERNEL_MASK_SC + 4] {
        assert_eq!(
            at_4_93(&request_at(0, 0, 0x7ff, pc), &uart),
            set(params(0, 0x7ff, 0, 0)),
            "pc {pc:#x}"
        );
    }
}

#[test]
fn a_set_param_call_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let r = request(0, 0, 0x7ff);
    let what = "a virtual UART parameter write at a firmware whose lv1 handler is not read";
    for firmware in [None, Some("4.92")] {
        assert_eq!(
            set_param(&r, firmware, &UartState::default()),
            refused(&r, what),
            "{firmware:?}"
        );
    }
}
