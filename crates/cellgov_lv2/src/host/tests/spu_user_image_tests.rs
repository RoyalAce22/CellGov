//! `sys_spu_thread_initialize` with a user-type `sys_spu_image`: the
//! segment-table gates, the snapshot group start loads, and the
//! withdraw on a refused initialize.

use crate::dispatch::{Lv2Dispatch, SpuLoadImage};
use crate::host::test_support::FakeRuntime;
use crate::host::Lv2Host;
use crate::image::LsSegment;
use crate::request::Lv2Request;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_spu::{image, segment};

const IMG: u32 = 0x200;
const SEGS: u32 = 0x300;
const COPY_SRC: u32 = 0x1000;
const ENTRY: u32 = 0xd0;

struct Seg {
    kind: u32,
    ls: u32,
    size: u32,
    addr: u32,
}

fn copy(ls: u32, size: u32, addr: u32) -> Seg {
    Seg {
        kind: segment::TYPE_COPY,
        ls,
        size,
        addr,
    }
}

fn fill(ls: u32, size: u32, value: u32) -> Seg {
    Seg {
        kind: segment::TYPE_FILL,
        ls,
        size,
        addr: value,
    }
}

fn info(size: u32) -> Seg {
    Seg {
        kind: segment::TYPE_INFO,
        ls: 0,
        size,
        addr: 0,
    }
}

fn commit(mem: &mut GuestMemory, addr: u32, bytes: &[u8]) {
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(addr)), bytes.len() as u64).unwrap(),
        bytes,
    )
    .unwrap();
}

/// Seed a user-type record at `IMG` whose table at `SEGS` holds
/// `segs`, with `COPY_SRC..+0x40` filled with a ramp.
fn runtime(entry: u32, nsegs: i32, segs: &[Seg]) -> FakeRuntime {
    runtime_typed(image::TYPE_USER, entry, nsegs, segs)
}

fn runtime_typed(image_type: u32, entry: u32, nsegs: i32, segs: &[Seg]) -> FakeRuntime {
    let mut mem = GuestMemory::new(0x4000);
    let mut record = [0u8; 16];
    record[0..4].copy_from_slice(&image_type.to_be_bytes());
    record[4..8].copy_from_slice(&entry.to_be_bytes());
    record[8..12].copy_from_slice(&SEGS.to_be_bytes());
    record[12..16].copy_from_slice(&nsegs.to_be_bytes());
    commit(&mut mem, IMG, &record);
    for (i, seg) in segs.iter().enumerate() {
        let mut row = [0u8; segment::LEN as usize];
        row[0..4].copy_from_slice(&seg.kind.to_be_bytes());
        row[4..8].copy_from_slice(&seg.ls.to_be_bytes());
        row[8..12].copy_from_slice(&seg.size.to_be_bytes());
        let addr_at = segment::ADDR_OFFSET as usize;
        row[addr_at..addr_at + 4].copy_from_slice(&seg.addr.to_be_bytes());
        commit(&mut mem, SEGS + i as u32 * segment::LEN, &row);
    }
    let ramp: Vec<u8> = (0..0x40u8).collect();
    commit(&mut mem, COPY_SRC, &ramp);
    FakeRuntime::with_memory(mem)
}

/// Seed only the record at `IMG`, pointing its table at `segs_ptr`;
/// nothing is written there.
fn record_runtime(entry: u32, segs_ptr: u32, nsegs: i32) -> FakeRuntime {
    let mut mem = GuestMemory::new(0x4000);
    let mut record = [0u8; 16];
    record[0..4].copy_from_slice(&image::TYPE_USER.to_be_bytes());
    record[4..8].copy_from_slice(&entry.to_be_bytes());
    record[8..12].copy_from_slice(&segs_ptr.to_be_bytes());
    record[12..16].copy_from_slice(&nsegs.to_be_bytes());
    commit(&mut mem, IMG, &record);
    FakeRuntime::with_memory(mem)
}

/// Seed a kernel-shaped record at `IMG` naming `handle`.
fn kernel_runtime(handle: u32) -> FakeRuntime {
    runtime_typed(image::TYPE_KERNEL, handle, 0, &[])
}

fn host_with_group(rt: &FakeRuntime, num_threads: u32) -> Lv2Host {
    let mut host = Lv2Host::new();
    let d = host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x100,
            num_threads,
            priority: 0,
            attr_ptr: 0,
        },
        UnitId::new(0),
        rt,
    );
    assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }), "{d:?}");
    host
}

fn initialize(host: &mut Lv2Host, rt: &FakeRuntime, group_id: u32, slot: u32) -> u64 {
    match host.dispatch(
        Lv2Request::SpuThreadInitialize {
            thread_ptr: 0x180,
            group_id,
            thread_num: slot,
            img_ptr: IMG,
            attr_ptr: 0,
            arg_ptr: 0,
        },
        UnitId::new(0),
        rt,
    ) {
        Lv2Dispatch::Immediate { code, .. } => code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn a_user_image_is_snapshotted_at_initialize_and_loaded_at_group_start() {
    let rt = runtime(
        ENTRY,
        3,
        &[
            copy(0x100, 0x20, COPY_SRC + 0x10),
            fill(0x200, 0x10, 0xdead_beef),
            info(0x40),
        ],
    );
    let mut host = host_with_group(&rt, 1);
    assert_eq!(initialize(&mut host, &rt, 1, 0), 0);
    assert_eq!(host.content_store().user_image_count(), 1);

    let d = host.dispatch(
        Lv2Request::SpuThreadGroupStart { group_id: 1 },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::RegisterSpu { inits, code: 0, .. } = d else {
        panic!("expected RegisterSpu, got {d:?}");
    };
    let init = &inits[&0];
    assert_eq!(init.entry_pc, ENTRY);
    assert_eq!(
        init.image,
        SpuLoadImage::Segments(vec![
            LsSegment {
                ls_start: 0x100,
                bytes: (0x10..0x30u8).collect(),
            },
            LsSegment {
                ls_start: 0x200,
                bytes: [0xde, 0xad, 0xbe, 0xef].repeat(4),
            },
        ])
    );
}

#[test]
fn a_user_image_without_a_copy_segment_is_einval() {
    let rt = runtime(ENTRY, 2, &[fill(0x100, 0x10, 0), info(0)]);
    let mut host = host_with_group(&rt, 1);
    assert_eq!(
        initialize(&mut host, &rt, 1, 0),
        u64::from(cell_errors::CELL_EINVAL)
    );
    assert_eq!(host.content_store().user_image_count(), 0);
}

#[test]
fn overlapping_loadable_segments_are_einval() {
    let rt = runtime(
        ENTRY,
        2,
        &[copy(0x100, 0x20, COPY_SRC), copy(0x110, 0x10, COPY_SRC)],
    );
    let mut host = host_with_group(&rt, 1);
    assert_eq!(
        initialize(&mut host, &rt, 1, 0),
        u64::from(cell_errors::CELL_EINVAL)
    );
}

#[test]
fn an_info_segment_overlapping_a_copy_is_not_a_conflict() {
    let rt = runtime(ENTRY, 2, &[info(0x10), copy(0x0, 0x20, COPY_SRC)]);
    let mut host = host_with_group(&rt, 1);
    assert_eq!(initialize(&mut host, &rt, 1, 0), 0);
}

#[test]
fn a_misaligned_ls_or_size_and_an_out_of_store_segment_are_einval() {
    let einval = u64::from(cell_errors::CELL_EINVAL);
    for seg in [
        copy(0x108, 0x10, COPY_SRC),
        copy(0x100, 0x18, COPY_SRC),
        copy(0x100, 0, COPY_SRC),
        copy(0x4_0000, 0x10, COPY_SRC),
        copy(0x100, 0x4_0010, COPY_SRC),
        copy(0x3_fff0, 0x20, COPY_SRC),
        copy(0x100, 0x10, COPY_SRC + 2),
    ] {
        let rt = runtime(ENTRY, 1, &[seg]);
        let mut host = host_with_group(&rt, 1);
        assert_eq!(initialize(&mut host, &rt, 1, 0), einval);
    }
}

#[test]
fn the_record_bounds_are_gated_before_the_table_is_read() {
    let einval = u64::from(cell_errors::CELL_EINVAL);
    for (entry, nsegs) in [
        (image::ENTRY_MAX + 1, 1),
        (ENTRY, 0),
        (ENTRY, -1),
        (ENTRY, image::NSEGS_MAX + 1),
    ] {
        let rt = runtime(entry, nsegs, &[copy(0x100, 0x10, COPY_SRC)]);
        let mut host = host_with_group(&rt, 1);
        assert_eq!(initialize(&mut host, &rt, 1, 0), einval);
    }
    let rt = runtime(
        image::ENTRY_MAX,
        image::NSEGS_MAX,
        &[copy(0x100, 0x10, COPY_SRC)],
    );
    let mut host = host_with_group(&rt, 1);
    // NSEGS_MAX rows are declared but only one is seeded; the rest read
    // as zeroed rows, which are unknown-type refusals.
    assert_eq!(initialize(&mut host, &rt, 1, 0), einval);

    // A table that cannot be read is never reached when the record
    // itself is refused; the same table with a legal count is EFAULT.
    let rt = record_runtime(ENTRY, 0x3ff0, 0);
    let mut host = host_with_group(&rt, 1);
    assert_eq!(initialize(&mut host, &rt, 1, 0), einval);
    let rt = record_runtime(ENTRY, 0x3ff0, 1);
    let mut host = host_with_group(&rt, 1);
    assert_eq!(
        initialize(&mut host, &rt, 1, 0),
        u64::from(cell_errors::CELL_EFAULT)
    );
}

#[test]
fn a_record_at_the_top_of_the_address_space_is_efault() {
    let rt = runtime(ENTRY, 1, &[copy(0x100, 0x10, COPY_SRC)]);
    let mut host = host_with_group(&rt, 1);
    for img_ptr in [u32::MAX - 3, u32::MAX - 1, u32::MAX] {
        let d = host.dispatch(
            Lv2Request::SpuThreadInitialize {
                thread_ptr: 0x180,
                group_id: 1,
                thread_num: 0,
                img_ptr,
                attr_ptr: 0,
                arg_ptr: 0,
            },
            UnitId::new(0),
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = d else {
            panic!("expected Immediate, got {d:?}");
        };
        assert_eq!(
            code,
            u64::from(cell_errors::CELL_EFAULT),
            "img_ptr {img_ptr:#x}"
        );
    }
    // A table row at the top of the space is refused the same way.
    let rt = record_runtime(ENTRY, u32::MAX - 0x17, 1);
    let mut host = host_with_group(&rt, 1);
    assert_eq!(
        initialize(&mut host, &rt, 1, 0),
        u64::from(cell_errors::CELL_EFAULT)
    );
}

#[test]
fn the_segment_record_layout_is_24_bytes_with_the_source_word_at_0x10() {
    assert_eq!(segment::LEN, 0x18);
    assert_eq!(
        (
            segment::TYPE_OFFSET,
            segment::LS_OFFSET,
            segment::SIZE_OFFSET,
            segment::ADDR_OFFSET
        ),
        (0, 4, 8, 0x10)
    );
    assert_eq!(image::LEN, 16);
    // A table written with a 16-byte stride puts row 1's type word
    // where row 0's source word is read, so a parse using the 24-byte
    // stride refuses the table.
    let mut mem = GuestMemory::new(0x4000);
    let mut record = [0u8; 16];
    record[0..4].copy_from_slice(&image::TYPE_USER.to_be_bytes());
    record[4..8].copy_from_slice(&ENTRY.to_be_bytes());
    record[8..12].copy_from_slice(&SEGS.to_be_bytes());
    record[12..16].copy_from_slice(&2i32.to_be_bytes());
    commit(&mut mem, IMG, &record);
    for (i, (ls, addr)) in [(0x100u32, COPY_SRC), (0x200, COPY_SRC + 2)]
        .into_iter()
        .enumerate()
    {
        let mut row = [0u8; 16];
        row[0..4].copy_from_slice(&segment::TYPE_COPY.to_be_bytes());
        row[4..8].copy_from_slice(&ls.to_be_bytes());
        row[8..12].copy_from_slice(&0x10u32.to_be_bytes());
        row[12..16].copy_from_slice(&addr.to_be_bytes());
        commit(&mut mem, SEGS + i as u32 * 16, &row);
    }
    let rt = FakeRuntime::with_memory(mem);
    let mut host = host_with_group(&rt, 1);
    // Row 0's source word reads row 1's type (COPY = 1), which is not
    // 4-byte aligned.
    assert_eq!(
        initialize(&mut host, &rt, 1, 0),
        u64::from(cell_errors::CELL_EINVAL)
    );
    assert_eq!(host.content_store().user_image_count(), 0);
}

#[test]
fn a_kernel_record_naming_an_unknown_id_is_esrch_at_initialize() {
    let rt = kernel_runtime(5);
    let mut host = host_with_group(&rt, 1);
    assert_eq!(
        initialize(&mut host, &rt, 1, 0),
        u64::from(cell_errors::CELL_ESRCH)
    );
    assert!(host.thread_groups().get(1).unwrap().slots.is_empty());
}

#[test]
fn a_kernel_record_naming_a_user_image_handle_is_esrch() {
    let user_rt = runtime(ENTRY, 1, &[copy(0x100, 0x10, COPY_SRC)]);
    let mut host = host_with_group(&user_rt, 2);
    assert_eq!(initialize(&mut host, &user_rt, 1, 0), 0);
    let user_handle = host.thread_groups().get(1).unwrap().slots[&0].image_handle;
    let forged = kernel_runtime(user_handle.raw());
    assert_eq!(
        initialize(&mut host, &forged, 1, 1),
        u64::from(cell_errors::CELL_ESRCH)
    );
    assert_eq!(host.thread_groups().get(1).unwrap().slots.len(), 1);
    assert_eq!(host.content_store().user_image_count(), 1);
}

#[test]
fn destroying_a_group_withdraws_its_user_images_and_keeps_kernel_images() {
    let user_rt = runtime(ENTRY, 1, &[copy(0x100, 0x10, COPY_SRC)]);
    let mut host = host_with_group(&user_rt, 2);
    let kernel = host.content_store_mut().register(b"/spu.elf", vec![0xAA]);
    let kernel_rt = kernel_runtime(kernel.raw());
    assert_eq!(initialize(&mut host, &user_rt, 1, 0), 0);
    assert_eq!(initialize(&mut host, &kernel_rt, 1, 1), 0);
    assert_eq!(host.content_store().user_image_count(), 1);
    assert_eq!(host.content_store().len(), 1);

    let d = host.dispatch(
        Lv2Request::SpuThreadGroupDestroy { id: 1 },
        UnitId::new(0),
        &user_rt,
    );
    assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }), "{d:?}");
    assert_eq!(host.content_store().user_image_count(), 0);
    assert_eq!(host.content_store().len(), 1);
    assert!(host.content_store().lookup_by_handle(kernel).is_some());
}

#[test]
fn a_second_or_oversized_info_segment_is_einval() {
    let einval = u64::from(cell_errors::CELL_EINVAL);
    let rt = runtime(ENTRY, 3, &[copy(0x100, 0x10, COPY_SRC), info(0), info(0)]);
    let mut host = host_with_group(&rt, 1);
    assert_eq!(initialize(&mut host, &rt, 1, 0), einval);
    let rt = runtime(
        ENTRY,
        2,
        &[
            copy(0x100, 0x10, COPY_SRC),
            info(segment::INFO_SIZE_MAX + 1),
        ],
    );
    let mut host = host_with_group(&rt, 1);
    assert_eq!(initialize(&mut host, &rt, 1, 0), einval);
}

#[test]
fn an_unreadable_copy_source_is_efault_and_registers_nothing() {
    let rt = runtime(ENTRY, 1, &[copy(0x100, 0x10, 0x3ff8)]);
    let mut host = host_with_group(&rt, 1);
    assert_eq!(
        initialize(&mut host, &rt, 1, 0),
        u64::from(cell_errors::CELL_EFAULT)
    );
    assert_eq!(host.content_store().user_image_count(), 0);
}

#[test]
fn a_refused_slot_withdraws_the_user_image_it_registered() {
    let rt = runtime(ENTRY, 1, &[copy(0x100, 0x10, COPY_SRC)]);
    let mut host = host_with_group(&rt, 1);
    assert_eq!(
        initialize(&mut host, &rt, 7, 0),
        u64::from(cell_errors::CELL_ESRCH)
    );
    assert_eq!(host.content_store().user_image_count(), 0);
    assert_eq!(initialize(&mut host, &rt, 1, 0), 0);
    assert_eq!(
        initialize(&mut host, &rt, 1, 0),
        u64::from(cell_errors::CELL_EBUSY)
    );
    assert_eq!(host.content_store().user_image_count(), 1);
}

#[test]
fn an_unknown_image_type_is_einval() {
    let rt = runtime_typed(2, ENTRY, 1, &[copy(0x100, 0x10, COPY_SRC)]);
    let mut host = host_with_group(&rt, 1);
    assert_eq!(
        initialize(&mut host, &rt, 1, 0),
        u64::from(cell_errors::CELL_EINVAL)
    );
}
