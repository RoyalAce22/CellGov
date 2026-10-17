//! `sys_cond_create` answers the `lv2_cond_pshared` probe's six creates
//! as the reference console does: a process-shared cond binds only a
//! process-shared mutex, and carries a nonzero key and an attach flag of
//! 1 to 3. The same return code, id store and destroy answer, read from
//! the committed capture.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::sync::{
    SYS_SYNC_FIFO, SYS_SYNC_NOT_ADAPTIVE, SYS_SYNC_NOT_PROCESS_SHARED, SYS_SYNC_NOT_RECURSIVE,
    SYS_SYNC_PROCESS_SHARED,
};
use cellgov_ps3_abi::lv2::syscall::{COND_CREATE, COND_DESTROY, MUTEX_CREATE, MUTEX_DESTROY};

use super::*;
use crate::host::test_support::{console_payload, payload_u32, seed_primary_ppu, FakeRuntime};
use crate::request::classify;

const PROBE: &str = "lv2_cond_pshared";
const NOT_RUN: u32 = 0xFFFF_FFFF;
const ID_PTR: u64 = 0x100;
const BLOCK: u64 = 0x800;
/// The probe's process-shared key high word.
const KEY_HI: u32 = 0x4347_4F00;

/// The answer to `number` with `args` against a runtime holding `block`
/// at BLOCK, and the id it stored, if any.
fn call(host: &mut Lv2Host, number: u64, args: [u64; 4], block: &[u32]) -> (u32, Option<u32>) {
    let mut mem = GuestMemory::new(0x2000);
    if !block.is_empty() {
        let bytes: Vec<u8> = block.iter().flat_map(|w| w.to_be_bytes()).collect();
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(BLOCK), bytes.len() as u64).expect("in range"),
            &bytes,
        )
        .expect("mapped");
    }
    let rt = FakeRuntime::with_memory(mem);
    let mut words = [0; 8];
    words[..4].copy_from_slice(&args);
    match host.dispatch(classify(number, &words), UnitId::new(0), &rt) {
        Lv2Dispatch::Immediate { code, effects } => {
            let id = effects.iter().find_map(|effect| match effect {
                cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. }
                    if range.start().raw() == ID_PTR =>
                {
                    Some(u32::from_be_bytes(
                        bytes.bytes().try_into().expect("4 bytes"),
                    ))
                }
                _ => None,
            });
            (code as u32, id)
        }
        other => panic!("call {number}: unexpected dispatch {other:?}"),
    }
}

/// The probe's mutex: FIFO, not recursive, not adaptive, and when
/// process-shared, key `KEY_HI:key_lo` with flags 1.
fn mutex(host: &mut Lv2Host, pshared: u32, key_lo: u32) -> u64 {
    let shared = pshared == SYS_SYNC_PROCESS_SHARED;
    let (hi, lo, flags) = if shared {
        (KEY_HI, key_lo, 1)
    } else {
        (0, 0, 0)
    };
    let block = [
        SYS_SYNC_FIFO,
        SYS_SYNC_NOT_RECURSIVE,
        pshared,
        SYS_SYNC_NOT_ADAPTIVE,
        hi,
        lo,
        flags,
        0,
        0,
        0,
    ];
    let (code, id) = call(host, MUTEX_CREATE, [ID_PTR, BLOCK, 0, 0], &block);
    assert_eq!(code, 0, "the probe's mutex creates succeed");
    u64::from(id.expect("the mutex id"))
}

#[test]
fn sys_cond_create_answers_each_process_shared_case_as_the_console_does() {
    let payload = console_payload(PROBE);
    assert_eq!(payload.len(), 16 + 16 * 6);
    assert_eq!(payload_u32(&payload, 0), 0, "no step of the probe failed");

    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, UnitId::new(0));
    let m1 = mutex(&mut host, SYS_SYNC_NOT_PROCESS_SHARED, 0);
    let m2 = mutex(&mut host, SYS_SYNC_PROCESS_SHARED, 0x100);

    // [mutex, pshared, key hi, key lo, flags], in payload order.
    let cases = [
        (m1, SYS_SYNC_PROCESS_SHARED, KEY_HI, 0, 1),
        (m2, SYS_SYNC_PROCESS_SHARED, KEY_HI, 1, 1),
        (m2, SYS_SYNC_PROCESS_SHARED, KEY_HI, 2, 0),
        (m2, SYS_SYNC_PROCESS_SHARED, KEY_HI, 3, 3),
        (m2, SYS_SYNC_NOT_PROCESS_SHARED, 0, 0, 0),
        (m2, SYS_SYNC_PROCESS_SHARED, 0, 0, 1),
    ];
    let mut differing = Vec::new();
    for (k, &(m, pshared, hi, lo, flags)) in cases.iter().enumerate() {
        let block = [pshared, flags, hi, lo, 0, 0];
        let (code, id) = call(&mut host, COND_CREATE, [ID_PTR, m, BLOCK, 0], &block);
        let destroy = match id {
            Some(id) if code == 0 => call(&mut host, COND_DESTROY, [u64::from(id), 0, 0, 0], &[]).0,
            _ => NOT_RUN,
        };
        let at = 16 + 16 * k;
        let cellgov = [code, u32::from(id.is_some()), destroy];
        let console = [
            payload_u32(&payload, at),
            payload_u32(&payload, at + 4),
            payload_u32(&payload, at + 8),
        ];
        if cellgov != console {
            differing.push(format!(
                "case {k}: cellgov {cellgov:x?}, console {console:x?}"
            ));
        }
    }
    assert!(
        differing.is_empty(),
        "[create rc, id written, destroy rc] differs:\n{}",
        differing.join("\n")
    );

    let (m1_destroy, _) = call(&mut host, MUTEX_DESTROY, [m1, 0, 0, 0], &[]);
    assert_eq!(m1_destroy, payload_u32(&payload, 8), "the M1 destroy");

    let (m2_destroy, _) = call(&mut host, MUTEX_DESTROY, [m2, 0, 0, 0], &[]);
    assert_eq!(m2_destroy, payload_u32(&payload, 12), "the M2 destroy");
}
