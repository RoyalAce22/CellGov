//! `_sys_prx_get_module_list` (494): where the `idlist` fill stops.

use super::*;

/// Guest memory holding a modelled option struct at `p_info` that
/// declares `max` slots and points `idlist` at `idlist_ptr`.
fn memory_with_pinfo(p_info: u32, max: u32, idlist_ptr: u32) -> cellgov_mem::GuestMemory {
    use cellgov_ps3_abi::lv2::prx::get_module_list_option as opt;
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let mut image = [0u8; 0x20];
    image[0..8].copy_from_slice(&opt::SIZE.to_be_bytes());
    let max_at = opt::MAX_OFFSET as usize;
    let idlist_at = opt::IDLIST_OFFSET as usize;
    image[max_at..max_at + 4].copy_from_slice(&max.to_be_bytes());
    image[idlist_at..idlist_at + 4].copy_from_slice(&idlist_ptr.to_be_bytes());
    mem.apply_commit(
        cellgov_mem::ByteRange::new(
            cellgov_mem::GuestAddr::new(u64::from(p_info)),
            image.len() as u64,
        )
        .unwrap(),
        &image,
    )
    .unwrap();
    mem
}

/// A host holding one module the fill reports, under a registry that
/// also holds liblv2.sprx (which the fill filters out).
fn host_with_one_listed_module() -> Lv2Host {
    let mut host = Lv2Host::new();
    host.prx_registry_mut().register(
        "liblv2".into(),
        "liblv2".into(),
        0x0145_0000,
        0x0146_0000,
        0x0145_d000,
        None,
        None,
    );
    host.prx_registry_mut().register(
        "libaudio".into(),
        "cellAudio_Library".into(),
        0x0147_0000,
        0x0148_0000,
        0x0147_da30,
        None,
        None,
    );
    host
}

fn module_list(host: &mut Lv2Host, p_info: u32, rt: &FakeRuntime) -> Lv2Dispatch {
    use cellgov_ps3_abi::lv2::prx::get_module_list_option as opt;
    host.dispatch(
        Lv2Request::Unsupported {
            number: cellgov_ps3_abi::lv2::syscall::SYS_PRX_GET_MODULE_LIST,
            args: [opt::FLAG_FILL_LIST, u64::from(p_info), 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        rt,
    )
}

#[test]
fn an_idlist_slot_off_the_top_of_the_address_space_stops_the_fill_and_is_named() {
    // Without the gate the slot address wraps and the write lands in
    // low guest memory under a fabricated address, behind CELL_OK.
    let mut host = host_with_one_listed_module();
    let rt = FakeRuntime::with_memory(memory_with_pinfo(0x4000, 8, u32::MAX - 1));
    let effects = match module_list(&mut host, 0x4000, &rt) {
        Lv2Dispatch::Immediate { code: 0, effects } => effects,
        other => panic!("expected Immediate{{code:0}}, got {other:?}"),
    };
    assert_eq!(
        host.invariant_break_site_count("dispatch.prx_module_list_idlist_slot_wraps"),
        1
    );
    assert_eq!(effects.len(), 1, "the count write, and no slot write");
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x4010);
            assert_eq!(u32::from_be_bytes(bytes.bytes().try_into().unwrap()), 0);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn an_idlist_the_slot_fits_inside_writes_it_and_names_nothing() {
    // The control uses the same registry and the same `max`, with room
    // for the slot. A gate that refused every address passes the test
    // above and fails this one.
    let mut host = host_with_one_listed_module();
    let rt = FakeRuntime::with_memory(memory_with_pinfo(0x4000, 8, 0x4040));
    let effects = match module_list(&mut host, 0x4000, &rt) {
        Lv2Dispatch::Immediate { code: 0, effects } => effects,
        other => panic!("expected Immediate{{code:0}}, got {other:?}"),
    };
    assert_eq!(
        host.invariant_break_site_count("dispatch.prx_module_list_idlist_slot_wraps"),
        0
    );
    assert_eq!(effects.len(), 2, "one slot write plus the count write");
}
