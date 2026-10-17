//! `sys_mmapper_unmap_shared_memory` (335) through the runtime: the
//! window leaves the caller's space, and a keyed window's view leaves
//! the shared-mapping bookkeeping.

use cellgov_mem::PageSize;
use cellgov_ps3_abi::lv2::syscall::{
    MMAPPER_ALLOCATE_SHARED_MEMORY, MMAPPER_MAP_SHARED_MEMORY, MMAPPER_UNMAP_SHARED_MEMORY,
};

use super::lv2_apply::{range4, reserve_mmapper_areas};
use super::*;
use crate::runtime::spaces::AddressSpaceId;

const MEM_ID_PTR: u64 = 0x4000;
const UNMAP_ID_PTR: u64 = 0x4010;
const MAP_ADDR: u64 = 0x5000_0000;

fn call(rt: &mut Runtime, unit: UnitId, number: u64, args: [u64; 4]) -> Option<u64> {
    let [a, b, c, d] = args;
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::Unsupported {
            number,
            args: [a, b, c, d, 0, 0, 0, 0],
        },
        unit,
    );
    rt.registry_mut().drain_syscall_return(unit)
}

fn read_u32(rt: &Runtime, space: AddressSpaceId, addr: u64) -> u32 {
    let bytes = rt
        .space_memory(space)
        .unwrap()
        .read(range4(addr))
        .expect("mapped");
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[test]
fn an_unmapped_window_leaves_the_callers_space_and_maps_again() {
    let mut rt = build(0x10000, 1, 100);
    let source = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let space = AddressSpaceId::new(1);
    rt.create_address_space(space).unwrap();
    rt.space_memory_mut(space)
        .unwrap()
        .install_region(0, 0x10000, "child", PageSize::Page64K)
        .unwrap();
    rt.assign_unit_space(source, space).unwrap();
    reserve_mmapper_areas(&mut rt, source, 0x4100);

    let keyless = cellgov_ps3_abi::lv2::memory::SYS_MMAPPER_NO_SHM_KEY;
    let flag_64k = cellgov_ps3_abi::lv2::memory::page_size::FLAG_64K;
    assert_eq!(
        call(
            &mut rt,
            source,
            MMAPPER_ALLOCATE_SHARED_MEMORY,
            [keyless, 0x10000, flag_64k, MEM_ID_PTR]
        ),
        Some(0)
    );
    let mem_id = read_u32(&rt, space, MEM_ID_PTR);
    let map = [MAP_ADDR, u64::from(mem_id), 0, 0];
    assert_eq!(
        call(&mut rt, source, MMAPPER_MAP_SHARED_MEMORY, map),
        Some(0)
    );
    assert!(rt
        .space_memory(space)
        .unwrap()
        .read(range4(MAP_ADDR))
        .is_some());

    assert_eq!(
        call(
            &mut rt,
            source,
            MMAPPER_UNMAP_SHARED_MEMORY,
            [MAP_ADDR, UNMAP_ID_PTR, 0, 0]
        ),
        Some(0)
    );
    assert_eq!(
        read_u32(&rt, space, UNMAP_ID_PTR),
        mem_id,
        "335 returns the segment"
    );
    assert!(
        rt.space_memory(space)
            .unwrap()
            .read(range4(MAP_ADDR))
            .is_none(),
        "the window is gone from the caller's space"
    );

    assert_eq!(
        call(&mut rt, source, MMAPPER_MAP_SHARED_MEMORY, map),
        Some(0)
    );
    assert!(rt
        .space_memory(space)
        .unwrap()
        .read(range4(MAP_ADDR))
        .is_some());
    assert_eq!(
        rt.lv2_host().observability().invariant_break_count,
        0,
        "neither the removal nor the second install logs a break"
    );
}

#[test]
fn a_detached_keyed_view_leaves_the_mapping_and_the_last_one_drops_it() {
    const KEY: u64 = 0x50;
    const SIZE: u64 = 0x1_0000;
    let mut rt = build(0x10000, 1, 100);
    let child = AddressSpaceId::new(1);
    rt.create_address_space(child).unwrap();
    rt.memory_mut()
        .install_region(MAP_ADDR, SIZE as usize, "shm", PageSize::Page64K)
        .unwrap();
    rt.space_memory_mut(child)
        .unwrap()
        .install_region(MAP_ADDR, SIZE as usize, "shm", PageSize::Page64K)
        .unwrap();
    rt.attach_keyed_shm_view(KEY, SIZE, AddressSpaceId::BOOT, MAP_ADDR);
    rt.attach_keyed_shm_view(KEY, SIZE, child, MAP_ADDR);
    let window = range4(MAP_ADDR);
    assert_eq!(
        rt.shared_alias_ranges_in(AddressSpaceId::BOOT, window),
        vec![window]
    );

    rt.detach_keyed_shm_view(KEY, child, MAP_ADDR);
    assert!(rt
        .shared_alias_ranges_in(AddressSpaceId::BOOT, window)
        .is_empty());
    assert!(rt.shared_alias_ranges_in(child, window).is_empty());
    assert!(rt.spaces.shared.get(KEY).is_some(), "one view is left");
    assert_eq!(
        rt.spaces.keyed_installs.get(&KEY).map(|e| e.1.clone()),
        Some(vec![(AddressSpaceId::BOOT, MAP_ADDR)])
    );

    rt.detach_keyed_shm_view(KEY, AddressSpaceId::BOOT, MAP_ADDR);
    assert!(rt.spaces.shared.get(KEY).is_none());
    assert!(!rt.spaces.keyed_installs.contains_key(&KEY));
}

/// The `lv2_mmapper_remap` capture's payload words.
fn remap_console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/lv2_mmapper_remap/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

/// The `lv2_mmapper_remap` probe through the runtime, its guest store a
/// direct memory write. Slot 3 holds an id and slot 11 an address, so
/// those two compare as the segment and as an offset from the area.
#[test]
fn a_segment_keeps_its_contents_across_an_unmap_as_on_the_console() {
    const MARK: u32 = 0x1234_5678;
    const SKIP: u64 = 0xDEAD_D00D;
    let console = remap_console_words();
    let boot = AddressSpaceId::BOOT;
    let mut rt = build(0x10000, 1, 100);
    let unit = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    reserve_mmapper_areas(&mut rt, unit, 0x4100);
    let a = MAP_ADDR;
    let word = |rt: &Runtime, window: u64| u64::from(read_u32(rt, boot, window + 0x10));
    let rc = |v: Option<u64>| v.expect("the call returns");

    let mut out = [SKIP; 17];
    out[16] = 0;
    out[0] = 0;
    out[1] = a;
    let keyless = cellgov_ps3_abi::lv2::memory::SYS_MMAPPER_NO_SHM_KEY;
    let flag_64k = cellgov_ps3_abi::lv2::memory::page_size::FLAG_64K;
    out[2] = rc(call(
        &mut rt,
        unit,
        MMAPPER_ALLOCATE_SHARED_MEMORY,
        [keyless, 0x1_0000, flag_64k, MEM_ID_PTR],
    ));
    let m = u64::from(read_u32(&rt, boot, MEM_ID_PTR));
    out[3] = m;

    out[4] = rc(call(
        &mut rt,
        unit,
        MMAPPER_MAP_SHARED_MEMORY,
        [a, m, 0x40000, 0],
    ));
    out[5] = word(&rt, a);
    rt.memory_mut()
        .apply_commit(range4(a + 0x10), &MARK.to_be_bytes())
        .expect("mapped");
    out[6] = rc(call(
        &mut rt,
        unit,
        MMAPPER_UNMAP_SHARED_MEMORY,
        [a, UNMAP_ID_PTR, 0, 0],
    ));

    let b = a + 0x2_0000;
    out[7] = rc(call(
        &mut rt,
        unit,
        MMAPPER_MAP_SHARED_MEMORY,
        [b, m, 0x40000, 0],
    ));
    out[8] = word(&rt, b);
    out[9] = rc(call(
        &mut rt,
        unit,
        MMAPPER_UNMAP_SHARED_MEMORY,
        [b, UNMAP_ID_PTR, 0, 0],
    ));

    out[10] = rc(call(
        &mut rt,
        unit,
        cellgov_ps3_abi::lv2::syscall::MMAPPER_SEARCH_AND_MAP,
        [a, m, 0x40000, UNMAP_ID_PTR],
    ));
    let x = u64::from(read_u32(&rt, boot, UNMAP_ID_PTR));
    out[11] = x;
    out[12] = word(&rt, x);
    out[13] = rc(call(
        &mut rt,
        unit,
        MMAPPER_UNMAP_SHARED_MEMORY,
        [x, UNMAP_ID_PTR, 0, 0],
    ));

    out[14] = rc(call(
        &mut rt,
        unit,
        cellgov_ps3_abi::lv2::syscall::MMAPPER_FREE_SHARED_MEMORY,
        [m, 0, 0, 0],
    ));
    out[15] = rc(call(
        &mut rt,
        unit,
        cellgov_ps3_abi::lv2::syscall::MMAPPER_FREE_ADDRESS,
        [a, 0, 0, 0],
    ));

    assert_eq!(console.len(), out.len(), "payload length");
    for slot in 0..out.len() {
        let (ours, theirs) = match slot {
            3 => continue,
            11 => (out[11] - out[1], console[11] - console[1]),
            _ => (out[slot], console[slot]),
        };
        assert_eq!(
            ours, theirs,
            "slot {slot}: CellGov {ours:#x}, console {theirs:#x}"
        );
    }
    assert_eq!(
        console[8],
        u64::from(MARK),
        "the console reads the word back"
    );
}
