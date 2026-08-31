//! `sys_mmapper_allocate_shared_memory_ext` (339): the exclusive
//! keyed create, the entry-table gates, and the key-probe loop
//! callers run on `CELL_EEXIST`.

use super::*;
use cellgov_mem::{GuestAddr, GuestMemory};
use cellgov_ps3_abi::sys_memory::{ext_entry, page_size, SYS_MMAPPER_NO_SHM_KEY};
use cellgov_ps3_abi::syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_EXT;

const ENTRIES: u32 = 0x4000;
const MEM_ID_PTR: u32 = 0x9000;
const KEY: u64 = 0x8000_4d49_4f32_3211;
const SIZE_64K: u64 = 0x2_0000;

fn rt_with_entries(types: &[u64]) -> FakeRuntime {
    let mut mem = GuestMemory::new(0x10000);
    for (i, ty) in types.iter().enumerate() {
        let addr = u64::from(ENTRIES + i as u32 * ext_entry::LEN + ext_entry::TYPE_OFFSET);
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(addr), 8).unwrap(),
            &ty.to_be_bytes(),
        )
        .unwrap();
    }
    FakeRuntime::with_memory(mem)
}

fn ext(
    host: &mut Lv2Host,
    rt: &FakeRuntime,
    key: u64,
    size: u64,
    flags: u64,
    entry_count: i64,
) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::Unsupported {
            number: MMAPPER_ALLOCATE_SHARED_MEMORY_EXT,
            args: [
                key,
                size,
                flags,
                u64::from(ENTRIES),
                entry_count as u64,
                u64::from(MEM_ID_PTR),
                0,
                0,
            ],
        },
        UnitId::new(0),
        rt,
    )
}

fn code_of(d: &Lv2Dispatch) -> u64 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn mem_id_of(d: &Lv2Dispatch) -> u32 {
    let Lv2Dispatch::Immediate { code: 0, effects } = d else {
        panic!("expected success, got {d:?}");
    };
    let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] else {
        panic!("expected SharedWriteIntent, got {:?}", effects[0]);
    };
    assert_eq!(range.start().raw(), u64::from(MEM_ID_PTR));
    u32::from_be_bytes(bytes.bytes().try_into().unwrap())
}

#[test]
fn a_fresh_key_mints_a_handle_and_registers_the_key() {
    let mut host = Lv2Host::new();
    let rt = rt_with_entries(&[0, 1, 3]);
    let d = ext(&mut host, &rt, KEY, SIZE_64K, page_size::FLAG_64K, 3);
    let id = mem_id_of(&d);
    assert_ne!(id, 0);
    assert_eq!(host.state.mmapper_ipc.get(&KEY), Some(&id));
    let handle = host.state.mmapper_handles.get(id).unwrap();
    assert_eq!(handle.size, SIZE_64K as u32);
    assert_eq!(handle.align, page_size::GRANULE_64K);
    assert_eq!(host.obs.system_ipc_witness.shm_creates, 0);
}

#[test]
fn a_key_in_the_system_ipc_namespace_counts_as_a_witnessed_create() {
    use cellgov_ps3_abi::system_ipc::SYSTEM_IPC_KEY_NAMESPACE;
    let mut host = Lv2Host::new();
    let rt = rt_with_entries(&[0]);
    let key = SYSTEM_IPC_KEY_NAMESPACE | 0x20;
    mem_id_of(&ext(&mut host, &rt, key, SIZE_64K, page_size::FLAG_64K, 1));
    assert_eq!(host.obs.system_ipc_witness.shm_creates, 1);
    assert_eq!(
        code_of(&ext(&mut host, &rt, key, SIZE_64K, page_size::FLAG_64K, 1)),
        u64::from(cell_errors::CELL_EEXIST)
    );
    assert_eq!(host.obs.system_ipc_witness.shm_creates, 1);
}

#[test]
fn a_registered_key_is_eexist_and_the_caller_probes_the_next_key() {
    let mut host = Lv2Host::new();
    let rt = rt_with_entries(&[0]);
    let first = mem_id_of(&ext(&mut host, &rt, KEY, SIZE_64K, page_size::FLAG_64K, 1));
    assert_eq!(
        code_of(&ext(&mut host, &rt, KEY, SIZE_64K, page_size::FLAG_64K, 1)),
        u64::from(cell_errors::CELL_EEXIST)
    );
    let second = mem_id_of(&ext(
        &mut host,
        &rt,
        KEY + 1,
        SIZE_64K,
        page_size::FLAG_64K,
        1,
    ));
    assert_ne!(first, second);
    assert_eq!(host.state.mmapper_ipc.get(&KEY), Some(&first));
    assert_eq!(host.state.mmapper_ipc.get(&(KEY + 1)), Some(&second));
}

#[test]
fn a_keyless_create_registers_nothing_and_repeats_without_colliding() {
    let mut host = Lv2Host::new();
    let rt = rt_with_entries(&[0]);
    for key in [SYS_MMAPPER_NO_SHM_KEY, 0] {
        let first = mem_id_of(&ext(&mut host, &rt, key, SIZE_64K, page_size::FLAG_64K, 1));
        let second = mem_id_of(&ext(&mut host, &rt, key, SIZE_64K, page_size::FLAG_64K, 1));
        assert_ne!(first, second);
        assert_eq!(host.state.mmapper_ipc.get(&key), None);
        // A keyless create still mints a handle: 334 / 337 reach the
        // segment through the handle table under the returned id.
        for id in [first, second] {
            let handle = host.state.mmapper_handles.get(id).expect("handle minted");
            assert_eq!(handle.size, SIZE_64K as u32);
        }
    }
    assert!(host.state.mmapper_ipc.is_empty());
    assert_eq!(host.obs.system_ipc_witness.shm_creates, 0);
    assert!(host.obs.system_ipc_witness.keys_touched.is_empty());
}

#[test]
fn the_size_and_flag_gates_fire_before_the_entry_table_is_read() {
    let mut host = Lv2Host::new();
    let rt = rt_with_entries(&[7]);
    let einval = u64::from(cell_errors::CELL_EINVAL);
    let ealign = u64::from(cell_errors::CELL_EALIGN);
    assert_eq!(
        code_of(&ext(&mut host, &rt, KEY, 0, page_size::FLAG_64K, 1)),
        ealign
    );
    assert_eq!(
        code_of(&ext(&mut host, &rt, KEY, SIZE_64K, 0x300, 1)),
        einval
    );
    assert_eq!(
        code_of(&ext(&mut host, &rt, KEY, 0x1_0000, page_size::FLAG_1M, 1)),
        ealign
    );
    assert_eq!(
        code_of(&ext(
            &mut host,
            &rt,
            KEY,
            SIZE_64K,
            page_size::FLAG_64K | 0x1,
            1
        )),
        einval
    );
    assert_eq!(
        code_of(&ext(&mut host, &rt, KEY, SIZE_64K, page_size::FLAG_64K, 0)),
        einval
    );
    assert_eq!(
        code_of(&ext(&mut host, &rt, KEY, SIZE_64K, page_size::FLAG_64K, -1)),
        einval
    );
    assert_eq!(
        code_of(&ext(
            &mut host,
            &rt,
            KEY,
            SIZE_64K,
            page_size::FLAG_64K,
            i64::from(ext_entry::MAX_COUNT) + 1
        )),
        einval
    );
    assert!(host.state.mmapper_ipc.is_empty());
}

#[test]
fn an_unknown_entry_type_is_eperm_and_registers_nothing() {
    let mut host = Lv2Host::new();
    let rt = rt_with_entries(&[0, 7]);
    assert_eq!(
        code_of(&ext(&mut host, &rt, KEY, SIZE_64K, page_size::FLAG_64K, 2)),
        u64::from(cell_errors::CELL_EPERM)
    );
    assert!(host.state.mmapper_ipc.is_empty());
    assert!(host.state.mmapper_handles.is_empty());
}

#[test]
fn the_privileged_entry_type_needs_64k_pages_and_debug_or_root() {
    let eperm = u64::from(cell_errors::CELL_EPERM);
    let rt = rt_with_entries(&[ext_entry::PRIVILEGED_TYPE]);
    // A user process is refused whatever the page size.
    let mut user = Lv2Host::new();
    assert_eq!(
        code_of(&ext(&mut user, &rt, KEY, SIZE_64K, page_size::FLAG_64K, 1)),
        eperm
    );
    assert!(user.state.mmapper_ipc.is_empty());
    // Root: the page-size condition decides.
    let mut root = Lv2Host::new();
    root.set_control_flags1(cellgov_ps3_abi::sce::CTRL_FLAGS1_ROOT_MASK);
    assert_eq!(
        code_of(&ext(&mut root, &rt, KEY, 0x10_0000, page_size::FLAG_1M, 1)),
        eperm
    );
    assert_eq!(code_of(&ext(&mut root, &rt, KEY, 0x10_0000, 0, 1)), eperm);
    assert_eq!(
        code_of(&ext(&mut root, &rt, KEY, SIZE_64K, page_size::FLAG_64K, 1)),
        0
    );
}

#[test]
fn an_unreadable_entry_table_is_efault() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x4000);
    assert_eq!(
        code_of(&ext(&mut host, &rt, KEY, SIZE_64K, page_size::FLAG_64K, 1)),
        u64::from(cell_errors::CELL_EFAULT)
    );
}

#[test]
fn a_null_mem_id_pointer_is_efault_after_the_entries_pass() {
    let mut host = Lv2Host::new();
    let rt = rt_with_entries(&[0]);
    let d = host.dispatch(
        Lv2Request::Unsupported {
            number: MMAPPER_ALLOCATE_SHARED_MEMORY_EXT,
            args: [
                KEY,
                SIZE_64K,
                page_size::FLAG_64K,
                u64::from(ENTRIES),
                1,
                0,
                0,
                0,
            ],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(code_of(&d), u64::from(cell_errors::CELL_EFAULT));
    assert!(host.state.mmapper_ipc.is_empty());
}
