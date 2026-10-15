//! `sys_uart_get_params` held to the `lv2_uart_get_params` capture from
//! the reference console: a game process, which has no root privilege
//! and did not itself initialize the UART, reads both ring sizes, and a
//! null output block faults. On the console vsh had already claimed the
//! UART; here no process claimed it.

use cellgov_event::UnitId;

use super::*;
use crate::host::test_support::{console_payload, FakeRuntime};
use crate::request::Lv2Request;
use crate::Lv2Dispatch;
use cellgov_effects::Effect;

const PARAMS_PTR: u32 = 0x100;

fn console() -> Vec<u64> {
    console_payload("lv2_uart_get_params")
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

#[test]
fn a_game_process_reads_both_ring_sizes_and_faults_on_a_null_block_as_on_the_console() {
    let words = console();
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    let get = |params_ptr| Lv2Request::UartGetParams { params_ptr };
    match host.dispatch(get(PARAMS_PTR), UnitId::new(0), &rt) {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, words[0], "rc");
            let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
                panic!("expected a write, got {:?}", effects[0]);
            };
            let out = bytes.bytes();
            assert_eq!(out.len(), 16, "two u64 sizes");
            assert_eq!(
                u64::from_be_bytes(out[..8].try_into().unwrap()),
                words[1],
                "rx"
            );
            assert_eq!(
                u64::from_be_bytes(out[8..].try_into().unwrap()),
                words[2],
                "tx"
            );
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
    assert_eq!(
        host.dispatch(get(0), UnitId::new(0), &rt),
        Lv2Dispatch::immediate(words[3]),
        "null block"
    );
}
