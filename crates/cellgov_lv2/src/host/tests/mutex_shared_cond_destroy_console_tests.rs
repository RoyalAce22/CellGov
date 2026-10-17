//! `sys_mutex_destroy` answers the `lv2_mutex_shared_cond_destroy`
//! probe's eight cases as the reference console does: a mutex with a
//! live cond refuses its destroy, and a cond that is not process-shared,
//! bound to a process-shared mutex, keeps that mutex from ever being
//! destroyed while it still locks and unlocks. Every word is read from
//! the committed capture.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::sync::{
    SYS_SYNC_FIFO, SYS_SYNC_NOT_ADAPTIVE, SYS_SYNC_NOT_PROCESS_SHARED, SYS_SYNC_NOT_RECURSIVE,
    SYS_SYNC_PROCESS_SHARED,
};
use cellgov_ps3_abi::lv2::syscall::{
    COND_CREATE, COND_DESTROY, MUTEX_CREATE, MUTEX_DESTROY, MUTEX_TRYLOCK, MUTEX_UNLOCK,
};

use super::*;
use crate::host::test_support::{console_payload, payload_u32, seed_primary_ppu, FakeRuntime};
use crate::request::classify;

const PROBE: &str = "lv2_mutex_shared_cond_destroy";
const NOT_RUN: u32 = 0xFFFF_FFFF;
const ID_PTR: u64 = 0x100;
const BLOCK: u64 = 0x800;
/// The probe's process-shared key high word.
const KEY_HI: u32 = 0x4347_4F00;
/// Marks a case that creates no cond.
const NONE: u32 = 0;

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

fn one(host: &mut Lv2Host, number: u64, id: u32) -> u32 {
    call(host, number, [u64::from(id), 0, 0, 0], &[]).0
}

/// One case, as the probe's `run_case` makes its calls: the eight words
/// it records.
fn run_case(
    host: &mut Lv2Host,
    k: u32,
    mshared: u32,
    cshared: u32,
    flags: u32,
    early: bool,
) -> [u32; 8] {
    let mut out = [NOT_RUN; 8];
    let shared = mshared == SYS_SYNC_PROCESS_SHARED;
    let block = [
        SYS_SYNC_FIFO,
        SYS_SYNC_NOT_RECURSIVE,
        mshared,
        SYS_SYNC_NOT_ADAPTIVE,
        if shared { KEY_HI } else { 0 },
        if shared { 0x200 + k } else { 0 },
        u32::from(shared),
        0,
        0,
        0,
    ];
    let (rc, m) = call(host, MUTEX_CREATE, [ID_PTR, BLOCK, 0, 0], &block);
    out[0] = rc;
    let Some(m) = m.filter(|_| rc == 0) else {
        return out;
    };
    let mut cond = None;
    if cshared != NONE {
        let cshared_on = cshared == SYS_SYNC_PROCESS_SHARED;
        let block = [
            cshared,
            flags,
            if cshared_on { KEY_HI } else { 0 },
            if cshared_on { 0x300 + k } else { 0 },
            0,
            0,
        ];
        let (rc, c) = call(host, COND_CREATE, [ID_PTR, u64::from(m), BLOCK, 0], &block);
        out[1] = rc;
        cond = c.filter(|_| rc == 0);
    }
    if let Some(c) = cond {
        if early {
            out[2] = one(host, MUTEX_DESTROY, m);
        }
        out[3] = one(host, COND_DESTROY, c);
    }
    out[4] = one(host, MUTEX_DESTROY, m);
    if out[4] == 0 {
        return out;
    }
    out[5] = one(host, MUTEX_DESTROY, m);
    out[6] = one(host, MUTEX_TRYLOCK, m);
    if out[6] == 0 {
        out[7] = one(host, MUTEX_UNLOCK, m);
    }
    out
}

#[test]
fn sys_mutex_destroy_answers_each_cond_binding_as_the_console_does() {
    let payload = console_payload(PROBE);
    assert_eq!(payload.len(), 8 * 32);

    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, UnitId::new(0));
    let p = SYS_SYNC_PROCESS_SHARED;
    let n = SYS_SYNC_NOT_PROCESS_SHARED;
    // [mutex pshared, cond pshared, cond flags, destroy the mutex while
    // the cond lives], in payload order.
    let cases = [
        (p, NONE, 0, false),
        (p, p, 1, false),
        (p, p, 3, false),
        (p, n, 0, false),
        (p, p, 0, false),
        (p, p, 1, true),
        (n, n, 0, false),
        (n, p, 1, false),
    ];
    let mut differing = Vec::new();
    let mut refused = 0;
    for (k, &(mshared, cshared, flags, early)) in cases.iter().enumerate() {
        let cellgov = run_case(&mut host, k as u32, mshared, cshared, flags, early);
        let console: [u32; 8] = std::array::from_fn(|i| payload_u32(&payload, 32 * k + 4 * i));
        if cellgov != console {
            differing.push(format!(
                "case {k}: cellgov {cellgov:x?}, console {console:x?}"
            ));
        }
        if console[4] != 0 {
            refused += 1;
        }
    }
    assert!(
        differing.is_empty(),
        "[mutex create, cond create, early destroy, cond destroy, destroy, again, trylock, unlock] differs:\n{}",
        differing.join("\n")
    );
    assert!(
        refused > 0,
        "the capture refuses at least one final destroy"
    );
}
