//! `sys_mutex_create` (100) answers the `lv2_mutex_create_attr`
//! probe's fourteen attribute blocks as the reference console does: the
//! same return code, the same id store, and the same destroy answer for
//! each, read from its committed capture.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::sync::{
    mutex_attribute as layout, SYS_SYNC_FIFO, SYS_SYNC_NOT_ADAPTIVE, SYS_SYNC_NOT_PROCESS_SHARED,
    SYS_SYNC_NOT_RECURSIVE, SYS_SYNC_PROCESS_SHARED,
};

use super::*;
use crate::host::test_support::{console_payload, payload_u32, seed_primary_ppu, FakeRuntime};
use crate::request::Lv2Request;

const PROBE: &str = "lv2_mutex_create_attr";

/// The probe's destroy word for a case whose create failed.
const NOT_RUN: u32 = 0xFFFF_FFFF;

const ATTR_PTR: u32 = 0x200;

const ID_PTR: u32 = 0x100;

/// One attribute block: the fields `ppu/main.c` sets per case.
#[derive(Clone, Copy)]
struct Attr {
    protocol: u32,
    recursive: u32,
    pshared: u32,
    adaptive: u32,
    ipc_key: u64,
    flags: u32,
}

const BASE: Attr = Attr {
    protocol: SYS_SYNC_FIFO,
    recursive: SYS_SYNC_NOT_RECURSIVE,
    pshared: SYS_SYNC_NOT_PROCESS_SHARED,
    adaptive: SYS_SYNC_NOT_ADAPTIVE,
    ipc_key: 0,
    flags: 0,
};

/// The probe's cases, in the order `ppu/main.c` runs them; `None` is
/// the null attribute pointer.
fn cases() -> [Option<Attr>; 14] {
    let shared = |ipc_key, flags| Attr {
        pshared: SYS_SYNC_PROCESS_SHARED,
        ipc_key,
        flags,
        ..BASE
    };
    [
        None,
        Some(BASE),
        Some(Attr {
            adaptive: 0,
            ..BASE
        }),
        Some(Attr {
            adaptive: 0x1000,
            ..BASE
        }),
        Some(Attr {
            adaptive: 0x3000,
            ..BASE
        }),
        Some(Attr {
            adaptive: 0xFFFF,
            ..BASE
        }),
        Some(Attr {
            adaptive: 0,
            protocol: 0,
            ..BASE
        }),
        Some(shared(0, 1)),
        Some(shared(0x1234, 0)),
        Some(shared(0x1234, 4)),
        Some(shared(0x1234, 1)),
        Some(Attr {
            adaptive: 0,
            ..shared(0, 1)
        }),
        Some(Attr {
            recursive: 0,
            adaptive: 0,
            ..BASE
        }),
        Some(Attr {
            ipc_key: 0x1234,
            flags: 4,
            ..BASE
        }),
    ]
}

fn attr_bytes(a: Attr) -> [u8; layout::SIZE as usize] {
    let mut bytes = [0u8; layout::SIZE as usize];
    for (off, word) in [
        (layout::PROTOCOL_OFFSET, a.protocol),
        (layout::RECURSIVE_OFFSET, a.recursive),
        (layout::PSHARED_OFFSET, a.pshared),
        (layout::ADAPTIVE_OFFSET, a.adaptive),
        (layout::FLAGS_OFFSET, a.flags),
    ] {
        bytes[off..off + 4].copy_from_slice(&word.to_be_bytes());
    }
    let key = layout::IPC_KEY_OFFSET;
    bytes[key..key + 8].copy_from_slice(&a.ipc_key.to_be_bytes());
    bytes
}

/// CellGov's `[create rc, id written, destroy rc]` for one case.
fn cellgov_case(attr: Option<Attr>) -> [u32; 3] {
    let mut mem = GuestMemory::new(0x1000);
    if let Some(a) = attr {
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(ATTR_PTR as u64), layout::SIZE as u64).unwrap(),
            &attr_bytes(a),
        )
        .unwrap();
    }
    let rt = FakeRuntime::with_memory(mem);
    let mut host = Lv2Host::new();
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: ID_PTR,
            attr_ptr: if attr.is_some() { ATTR_PTR } else { 0 },
        },
        src,
        &rt,
    );
    let Lv2Dispatch::Immediate { code, effects } = &created else {
        panic!("expected Immediate, got {created:?}");
    };
    let id = match effects.as_slice() {
        [] => None,
        [cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. }] => {
            assert_eq!(range.start().raw(), u64::from(ID_PTR));
            Some(u32::from_be_bytes(
                bytes.bytes().try_into().expect("4 bytes"),
            ))
        }
        other => panic!("expected at most one id write, got {other:?}"),
    };
    let destroy = match id {
        Some(mutex_id) if *code == 0 => {
            match host.dispatch(Lv2Request::MutexDestroy { mutex_id }, src, &rt) {
                Lv2Dispatch::Immediate { code, .. } => code as u32,
                other => panic!("expected Immediate, got {other:?}"),
            }
        }
        _ => NOT_RUN,
    };
    [*code as u32, u32::from(id.is_some()), destroy]
}

#[test]
fn mutex_create_answers_each_attribute_block_as_the_console_does() {
    let payload = console_payload(PROBE);
    let cases = cases();
    assert_eq!(payload.len(), 16 * cases.len(), "one record per case");
    for (i, attr) in cases.into_iter().enumerate() {
        let at = 16 * i;
        let console = [
            payload_u32(&payload, at),
            payload_u32(&payload, at + 4),
            payload_u32(&payload, at + 8),
        ];
        assert_eq!(
            cellgov_case(attr),
            console,
            "case {i}: [create rc, id written, destroy rc]"
        );
    }
}
