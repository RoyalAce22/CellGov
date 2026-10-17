//! `sys_memory_get_user_memory_size` held to the captures that read it
//! under different inputs: `lv2_memory_free_container` (64 KiB stack),
//! `lv2_user_memory_stack_1m` (1 MiB stack) and
//! `lv2_user_memory_big_image` (an image 8 MiB larger).
//!
//! The total is compared outright. The available memory is compared as
//! a change: the console charges the loaded image and the stacks to it,
//! which CellGov's counter leaves out.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::syscall;

use super::*;
use crate::host::test_support::{console_payload, FakeRuntime};
use crate::request::classify;
use crate::Lv2Dispatch;

const INFO_PTR: u64 = 0x100;
const CID_PTR: u64 = 0x108;
const MIB: u64 = 0x10_0000;

fn words(probe: &str) -> Vec<u64> {
    console_payload(probe)
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

/// The return code and the first write's bytes.
fn sc(host: &mut Lv2Host, rt: &FakeRuntime, number: u64, args: [u64; 2]) -> (u64, Vec<u8>) {
    let request = classify(number, &[args[0], args[1], 0, 0, 0, 0, 0, 0]);
    match host.dispatch(request, UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code, effects } => {
            let written = effects
                .iter()
                .find_map(|e| match e {
                    Effect::SharedWriteIntent { bytes, .. } => Some(bytes.bytes().to_vec()),
                    _ => None,
                })
                .unwrap_or_default();
            (code, written)
        }
        other => panic!("syscall {number}: expected Immediate, got {other:?}"),
    }
}

/// `(rc, total, available)` from one sc 352.
fn sizes(host: &mut Lv2Host, rt: &FakeRuntime) -> (u64, u64, u64) {
    let (code, w) = sc(
        host,
        rt,
        syscall::MEMORY_GET_USER_MEMORY_SIZE,
        [INFO_PTR, 0],
    );
    let word = |at: usize| u64::from(u32::from_be_bytes(w[at..at + 4].try_into().expect("u32")));
    (code, word(0), word(4))
}

#[test]
fn the_total_matches_the_console_whatever_the_stack_or_the_image() {
    let rt = FakeRuntime::new(0x1000);
    for probe in [
        "lv2_memory_free_container",
        "lv2_user_memory_stack_1m",
        "lv2_user_memory_big_image",
    ] {
        let w = words(probe);
        let (code, total, _) = sizes(&mut Lv2Host::new(), &rt);
        assert_eq!((code, total), (w[0], w[1]), "{probe}");
    }
}

#[test]
fn a_container_takes_its_size_from_the_total_and_the_available_memory_as_on_the_console() {
    let rt = FakeRuntime::new(0x1000);
    for probe in ["lv2_user_memory_stack_1m", "lv2_user_memory_big_image"] {
        let w = words(probe);
        let mut host = Lv2Host::new();
        let (_, _, first_avail) = sizes(&mut host, &rt);
        let (code, cid) = sc(
            &mut host,
            &rt,
            syscall::MEMORY_CONTAINER_CREATE,
            [CID_PTR, MIB],
        );
        assert_eq!(code, w[3], "{probe}: create");
        let (_, total, avail) = sizes(&mut host, &rt);
        assert_eq!(total, w[4], "{probe}: total with the container live");
        assert_eq!(
            first_avail - avail,
            w[2] - w[5],
            "{probe}: available taken by the container"
        );
        let cid = u64::from(u32::from_be_bytes(cid[..4].try_into().expect("cid")));
        let (code, _) = sc(&mut host, &rt, syscall::MEMORY_CONTAINER_DESTROY, [cid, 0]);
        assert_eq!(code, w[6], "{probe}: destroy");
        let (_, total, avail) = sizes(&mut host, &rt);
        assert_eq!(total, w[7], "{probe}: total after the destroy");
        assert_eq!(avail, first_avail, "{probe}: available after the destroy");
        assert_eq!(
            w[8], w[2],
            "{probe}: the console's available after the destroy"
        );
    }
}
