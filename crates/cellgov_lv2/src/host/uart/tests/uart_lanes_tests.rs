//! Every UART field moves the UART's sync-state term.

use super::*;
use crate::ppu_thread::PpuThreadId;

type UartChange = (&'static str, fn(&mut UartState));

#[test]
fn every_uart_field_moves_the_term() {
    let base = UartState::new();
    let changes: [UartChange; 11] = [
        ("initialized", |s| s.initialized = true),
        ("rx", |s| s.rx = vec![1]),
        ("rx byte", |s| s.rx = vec![0]),
        ("reader", |s| {
            s.readers.push_back(UartReader {
                thread: PpuThreadId::PRIMARY,
                buf_ptr: 0x100,
                size: 8,
            })
        }),
        ("av_cmd_ver", |s| s.av_cmd_ver = 1),
        ("hdmi_events", |s| s.hdmi_events = 1),
        ("hdmi_behavior", |s| s.hdmi_behavior = 0),
        ("head_b_initialized", |s| s.head_b_initialized = true),
        ("hdmi_res_set", |s| s.hdmi_res_set[1] = true),
        ("hdcp_first_auth", |s| s.hdcp_first_auth[0] = false),
        ("hdmi_to_state", |s| s.hdmi_to_state = 0),
    ];
    let mut seen = vec![base.sync_term()];
    for (name, change) in changes {
        let mut s = base.clone();
        change(&mut s);
        let term = s.sync_term();
        assert!(!seen.contains(&term), "{name}");
        seen.push(term);
    }
}

#[test]
fn readers_in_another_order_hash_differently() {
    let reader = |raw| UartReader {
        thread: PpuThreadId::new(raw),
        buf_ptr: 0x100,
        size: 8,
    };
    let mut a = UartState::new();
    a.readers.extend([reader(1), reader(2)]);
    let mut b = UartState::new();
    b.readers.extend([reader(2), reader(1)]);
    assert_ne!(a.sync_term(), b.sync_term());
}
