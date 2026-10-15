//! `_sys_prx_get_module_id_by_name` (496), against the answers the
//! `lv2_prx_module_id_by_name` console capture holds.

use super::*;

const NAME_AT: u32 = 0x4000;

/// `CELL_PRX_ERROR_UNKNOWN_MODULE` as the console capture holds it in r3.
const UNKNOWN: u64 = 0xffff_ffff_8001_112e;

fn host_with(name: &str) -> (Lv2Host, u32) {
    let mut host = Lv2Host::new();
    let id = host.prx_registry_mut().register(
        "lib".into(),
        name.into(),
        0x0147_0000,
        0x0148_0000,
        0x0147_da30,
        None,
        None,
    );
    (host, id)
}

/// Guest memory with `bytes` at [`NAME_AT`].
fn rt_with(bytes: &[u8]) -> FakeRuntime {
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    mem.apply_commit(
        cellgov_mem::ByteRange::new(
            cellgov_mem::GuestAddr::new(u64::from(NAME_AT)),
            bytes.len() as u64,
        )
        .unwrap(),
        bytes,
    )
    .unwrap();
    FakeRuntime::with_memory(mem)
}

fn id_by_name(host: &mut Lv2Host, rt: &FakeRuntime, name_ptr: u64) -> u64 {
    let request = Lv2Request::Unsupported {
        number: cellgov_ps3_abi::lv2::syscall::SYS_PRX_GET_MODULE_ID_BY_NAME,
        args: [name_ptr, 0, 0, 0, 0, 0, 0, 0],
    };
    match host.dispatch(request, UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code, .. } => code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn a_loaded_module_answers_its_id_and_any_other_name_is_unknown() {
    let (mut host, id) = host_with("liblv2");
    let at = u64::from(NAME_AT);
    let rt = rt_with(b"liblv2\0");
    assert_eq!(id_by_name(&mut host, &rt, at), u64::from(id));
    for name in [&b"cellSpursTuner\0"[..], b"liblv\0", b"LIBLV2\0", b"\0"] {
        let rt = rt_with(name);
        assert_eq!(id_by_name(&mut host, &rt, at), UNKNOWN, "{name:?}");
    }
}

#[test]
fn the_name_pointer_high_word_is_ignored() {
    let (mut host, id) = host_with("liblv2");
    let rt = rt_with(b"liblv2\0");
    assert_eq!(
        id_by_name(&mut host, &rt, 0x1_0000_0000 | u64::from(NAME_AT)),
        u64::from(id)
    );
}

#[test]
fn a_name_without_a_nul_in_its_first_28_bytes_compares_those_bytes() {
    let long = "cellLongModuleName_Library_With_A_Tail";
    let (mut host, id) = host_with(long);
    let rt = rt_with(&long.as_bytes()[..28]);
    assert_eq!(
        id_by_name(&mut host, &rt, u64::from(NAME_AT)),
        u64::from(id)
    );
}

#[test]
fn an_unreadable_name_is_einval() {
    let (mut host, _) = host_with("liblv2");
    let rt = FakeRuntime::new(0x1000);
    for ptr in [0, 0x8000] {
        assert_eq!(
            id_by_name(&mut host, &rt, ptr),
            u64::from(errno::CELL_EINVAL),
            "{ptr:#x}"
        );
    }
}
