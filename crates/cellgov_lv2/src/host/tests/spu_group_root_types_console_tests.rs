//! Holds group create's root-only types to the `spu_group_root_types`
//! capture from the reference console. The probe's EBOOT runs with root
//! permission, so the host here does too. The test makes the calls of
//! the probe, in its order, through the dispatch.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::spu::group_attr;

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;

const PROBE: &str = "spu_group_root_types";
const USER_PROBE: &str = "spu_group_root_types_user";
const CALLER: UnitId = UnitId::new(0);
/// The SELF control flags every probe EBOOT carries: root permission.
const PROBE_CONTROL_FLAGS1: u32 = 0x4000_0000;
/// The root-only type vsh creates its first group with.
const VSH_TYPE: u32 = 0x20A;

/// Create a group of `group_type`; returns the code the guest reads.
fn create(host: &mut Lv2Host, group_type: u32, num_threads: u32) -> u32 {
    let mut mem = GuestMemory::new(0x4000);
    mem.apply_commit(
        ByteRange::new(
            GuestAddr::new(u64::from(0x800 + group_attr::TYPE_OFFSET)),
            4,
        )
        .unwrap(),
        &group_type.to_be_bytes(),
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    match host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x400,
            num_threads,
            priority: 100,
            attr_ptr: 0x800,
        },
        CALLER,
        &rt,
    ) {
        Lv2Dispatch::Immediate { code, effects } => {
            if code == 0 {
                assert!(matches!(
                    effects.as_slice(),
                    [Effect::SharedWriteIntent { .. }]
                ));
            }
            code as u32
        }
        other => panic!("unexpected dispatch {other:?}"),
    }
}

#[test]
fn root_only_group_types_answer_as_the_console_does() {
    let payload = console_payload(PROBE);
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");

    let mut host = Lv2Host::new();
    host.set_control_flags1(PROBE_CONTROL_FLAGS1);
    assert!(host.boot_privilege().root());

    // A named divergence. On the console the first 0x20A group answers
    // EBUSY: the running system already holds the resource that type
    // takes, which a title's process cannot see past. In CellGov the
    // process that creates it is vsh itself, which must get it, so the
    // host answers 0.
    assert_eq!(
        u64::from(word(2)),
        u64::from(errno::CELL_EBUSY) & 0xFFFF_FFFF
    );
    assert_eq!(create(&mut host, VSH_TYPE, 1), 0);

    let got = [
        create(&mut host, VSH_TYPE, 2),
        create(&mut host, 0x002, 1),
        create(&mut host, 0x00A, 2),
        create(&mut host, 0x022, 2),
        create(&mut host, 0x102, 1),
        create(&mut host, 0x30A, 1),
    ];
    for (i, &code) in got.iter().enumerate() {
        let w = i + 3;
        assert_eq!(
            code,
            word(w),
            "w{w}: 0x{code:08x} vs console 0x{:08x}",
            word(w)
        );
    }
}

/// The same creates from a process without root permission, as the
/// `spu_group_root_types_user` probe makes them: its EBOOT is signed with
/// control_flags1 0, as a retail title is, and every root-only type is
/// refused.
#[test]
fn root_only_group_types_refuse_a_process_without_root_as_the_console_does() {
    let payload = console_payload(USER_PROBE);
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");

    let mut host = Lv2Host::new();
    host.set_control_flags1(0);
    assert!(!host.boot_privilege().root());

    let got = [
        create(&mut host, VSH_TYPE, 1),
        create(&mut host, VSH_TYPE, 2),
        create(&mut host, 0x002, 1),
        create(&mut host, 0x00A, 2),
        create(&mut host, 0x022, 2),
        create(&mut host, 0x102, 1),
        create(&mut host, 0x30A, 1),
    ];
    for (i, &code) in got.iter().enumerate() {
        let w = i + 2;
        assert_eq!(
            code,
            word(w),
            "w{w}: 0x{code:08x} vs console 0x{:08x}",
            word(w)
        );
    }
}
