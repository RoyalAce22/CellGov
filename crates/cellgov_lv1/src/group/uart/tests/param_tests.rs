use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's call for the receive buffer size
/// [OFW-4.93 lv2_kernel a:0x8000000000017730].
const KERNEL_RX_SC: u64 = 0x8000_0000_0001_7730;

/// The `sc` of the 4.93 kernel's call for the transmit buffer size
/// [OFW-4.93 lv2_kernel a:0x8000000000017750].
const KERNEL_TX_SC: u64 = 0x8000_0000_0001_7750;

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

/// The captured (receive, transmit) buffer sizes of port `port`.
fn kernel_port_sizes(port: u64) -> (u64, u64) {
    let (_, rx, tx) = kernel_ports()
        .into_iter()
        .find(|(p, ..)| *p == port)
        .unwrap_or_else(|| panic!("the capture holds no record of port {port}"));
    (rx, tx)
}

/// A thread 0 call of hypercall 165 for parameter `param` of port `port`.
///
/// The call is at one of the kernel's two `sc` addresses:
/// - `param` 5: the `sc` for the transmit buffer size.
/// - any other `param`: the `sc` for the receive buffer size.
fn request(port: u64, param: u64) -> HypercallRequest {
    let sc = if param == 5 {
        KERNEL_TX_SC
    } else {
        KERNEL_RX_SC
    };
    request_at(port, param, sc)
}

/// A thread 0 call of hypercall 165 at the `sc` address `pc`.
fn request_at(port: u64, param: u64, pc: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..2].copy_from_slice(&[port, param]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: GET_VIRTUAL_UART_PARAM,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(26_455),
    }
}

fn at_4_93(r: &HypercallRequest) -> HypercallDisposition {
    get_param(r, Some("4.93"))
}

/// lv1 returns 0 with the value in r4, and the vector's entry zeroes r5
/// to r10 [OFW-4.93 lv1 a:0x2F3B08-0x2F3B18] [OFW-4.93 lv1 a:0x0C08].
fn value(value: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, [value, 0, 0, 0, 0, 0, 0]))
}

fn lv1_r3(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: GET_VIRTUAL_UART_PARAM,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

#[test]
fn each_kernel_port_gets_its_captured_receive_and_transmit_buffer_sizes() {
    for (port, rx, tx) in kernel_ports() {
        assert_eq!(at_4_93(&request(port, 3)), value(rx), "port {port} rx");
        assert_eq!(at_4_93(&request(port, 5)), value(tx), "port {port} tx");
    }
}

/// The worker checks the port bound before the parameter bound
/// [OFW-4.93 lv1 a:0x2F34F8] [OFW-4.93 lv1 a:0x2F35C4-0x2F35C8].
#[test]
fn a_port_above_255_gets_lv1_s_no_port_r3_whatever_the_parameter() {
    for port in [256, 0x1_0000_0000, u64::MAX] {
        for param in [3, 8] {
            assert_eq!(
                at_4_93(&request(port, param)),
                lv1_r3(GET_VIRTUAL_UART_PARAM_NO_PORT_R3),
                "port {port:#x} param {param}"
            );
        }
    }
}

#[test]
fn a_parameter_above_7_of_a_kernel_port_gets_lv1_s_no_param_r3() {
    for (port, ..) in kernel_ports() {
        for param in [8, 0x1_0000_0003, u64::MAX] {
            assert_eq!(
                at_4_93(&request(port, param)),
                lv1_r3(GET_VIRTUAL_UART_PARAM_NO_PARAM_R3),
                "port {port} param {param:#x}"
            );
        }
    }
}

/// lv1 looks the port up before it checks the parameter
/// [OFW-4.93 lv1 a:0x2F34FC-0x2F35C8], so a port that no source gives is
/// a refusal even with a parameter above 7.
#[test]
fn a_port_that_no_source_gives_is_a_refusal() {
    let what = "a virtual UART port that no source shows lv1 holds or lacks";
    for port in [1, 3, 11, 255] {
        for param in [3, 5, 8] {
            let r = request(port, param);
            assert_eq!(at_4_93(&r), refused(&r, what), "port {port} param {param}");
        }
    }
}

#[test]
fn a_parameter_other_than_a_buffer_size_is_a_refusal() {
    let what = "a virtual UART parameter other than a buffer size";
    for param in [0, 1, 2, 4, 6, 7] {
        let r = request(0, param);
        assert_eq!(at_4_93(&r), refused(&r, what), "param {param}");
    }
}

/// lv1's code can link a port to another peer or replace its endpoint
/// while the partition runs [OFW-4.93 lv1 a:0x2EFE6C-0x2EFEC0]
/// [OFW-4.93 lv1 a:0x2F1A0C-0x2F1A1C], and the capture holds only the
/// answers to the kernel's two calls.
#[test]
fn a_buffer_size_call_at_a_pc_other_than_the_kernel_s_is_a_refusal() {
    let what = "a virtual UART buffer size call other than the GameOS kernel's";
    let calls = [
        (3, KERNEL_TX_SC),
        (5, KERNEL_RX_SC),
        (3, 0),
        (5, KERNEL_TX_SC + 4),
        (3, KERNEL_RX_SC - 4),
    ];
    for (port, ..) in kernel_ports() {
        for (param, pc) in calls {
            let r = request_at(port, param, pc);
            assert_eq!(
                at_4_93(&r),
                refused(&r, what),
                "port {port} param {param} pc {pc:#x}"
            );
        }
    }
}

/// The port and parameter checks read no state, so they answer at any pc.
#[test]
fn the_port_and_parameter_checks_answer_at_any_pc() {
    for pc in [0, KERNEL_RX_SC + 4] {
        assert_eq!(
            at_4_93(&request_at(256, 3, pc)),
            lv1_r3(GET_VIRTUAL_UART_PARAM_NO_PORT_R3),
            "pc {pc:#x}"
        );
        assert_eq!(
            at_4_93(&request_at(0, 8, pc)),
            lv1_r3(GET_VIRTUAL_UART_PARAM_NO_PARAM_R3),
            "pc {pc:#x}"
        );
    }
}

#[test]
fn a_param_call_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let r = request(0, 3);
    let what = "a virtual UART parameter call at a firmware whose lv1 handler is not read";
    for firmware in [None, Some("4.92")] {
        assert_eq!(get_param(&r, firmware), refused(&r, what), "{firmware:?}");
    }
}

/// The kernel's open function sends 165 for the receive buffer size, then
/// for the transmit buffer size
/// [OFW-4.93 lv2_kernel a:0x8000000000017714-0x8000000000017750].
#[test]
fn the_hypervisor_answers_the_kernel_s_two_calls_through_the_group_and_keeps_its_state() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let storage = GuestMemory::new(0x1000);
    let before = hv.groups().uart().clone();
    let (rx, tx) = kernel_port_sizes(0);
    let answers = [3, 5].map(|param| hv.dispatch_with(&request(0, param), &storage));
    assert_eq!(answers, [value(rx), value(tx)]);
    assert_eq!(hv.groups().uart(), &before);
}
