//! Goldens for a fresh runtime's sync-state hash.

use cellgov_mem::lanes::{
    additive_key, bytes_term, contribution, object_digest, source, LaneIndex,
};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

use crate::runtime::state::Runtime;

fn fresh() -> Runtime {
    Runtime::new(GuestMemory::new(16), Budget::new(4), 100)
}

/// Computed outside the crate from the SplitMix64 key streams of the
/// sync-state lanes and the mixer digest.
#[test]
fn fresh_runtime_sync_state_hash_wire_format_golden() {
    let rt = fresh();
    assert_eq!(rt.sync_state_hash(), 0x7699_0b37_018f_359d);
    assert_eq!(rt.sync_state_hash_from_scratch(), 0x7699_0b37_018f_359d);
}

/// The list holds every nonzero lane of a fresh runtime.
#[test]
fn fresh_runtime_sync_state_lane_vector_golden() {
    let lane = |src, object, field, slot, value| {
        contribution(LaneIndex::new(src, object, field, slot), value)
    };
    let present = |src, object| lane(src, object, 0, 0, 1);
    let lanes = [
        present(source::RSX_CURSOR, 0),
        present(source::RSX_FLIP, 0),
        present(source::RSX_SEM_OFFSET, 0),
        present(source::RSX_LABEL_BASE, 0),
        present(source::RSX_CALL_STACK, 0),
        present(source::LWMUTEX_IDS, 0),
        lane(source::LWMUTEX_IDS, 0, 1, 0, 1),
        present(source::GROUP_IDS, 0),
        lane(source::GROUP_IDS, 0, 1, 0, 1),
        present(source::PPU_THREAD_IDS, 0),
        lane(source::PPU_THREAD_IDS, 0, 1, 0, 1),
        lane(source::PPU_THREAD_IDS, 0, 2, 0, 0x0100_0001),
        present(source::PROCESS, 0x0100_0500),
        lane(source::PROCESS, 0x0100_0500, 1, 0, 0x0100_0300),
        lane(source::PROCESS, 0x0100_0500, 2, 0, 0x1010_0000_0100_0003),
        present(source::THREAD_STACKS, 0),
        present(source::PROCESS_COUNTS, 0),
        present(source::IMAGE_NEXT_HANDLE, 0),
        lane(source::IMAGE_NEXT_HANDLE, 0, 1, 0, 1),
        present(source::FS_NEXT_FD, 0),
        lane(source::FS_NEXT_FD, 0, 1, 0, 3),
        present(source::PRX_NEXT_ID, 0),
        lane(source::PRX_NEXT_ID, 0, 1, 0, 0x4002_0000),
        present(source::CONFIG_COUNTERS, 0),
        present(source::CONFIG_COUNTERS, 1),
        present(source::CONFIG_COUNTERS, 2),
        present(source::RSX_CONTEXT, 0),
        present(source::UART, 0),
        lane(source::UART, 0, 10, 0, 0xFF),
        lane(source::UART, 0, 13, 0, 1),
        lane(source::UART, 0, 13, 1, 1),
        lane(source::UART, 0, 14, 0, 2),
        bytes_term(source::UART, &[0, 3, 0], &[]),
        present(source::USBD, 0),
        present(source::KERNEL_CURSORS, 0),
        lane(source::KERNEL_CURSORS, 0, 1, 0, 0xD100_0001),
        present(source::KERNEL_CURSORS, 1),
        lane(source::KERNEL_CURSORS, 1, 1, 0, 0x0001_0000),
        present(source::KERNEL_CURSORS, 2),
        lane(source::KERNEL_CURSORS, 2, 1, 0, 0x5000_0000),
        present(source::KERNEL_CURSORS, 3),
        lane(source::KERNEL_CURSORS, 3, 1, 0, 0x3000_0000),
        present(source::KERNEL_CURSORS, 4),
        lane(source::KERNEL_CURSORS, 4, 1, 0, 1),
    ];
    let blobs = ["/app_home/PARAM.SFO", "/app_home/output.txt"].map(|path| {
        let object = object_digest(path.as_bytes());
        present(source::FS_BLOB, object).wrapping_add(lane(
            source::FS_BLOB,
            object,
            2,
            0,
            object_digest(&[]),
        ))
    });
    let sum = lanes
        .into_iter()
        .chain(blobs)
        .fold(additive_key(), u128::wrapping_add);
    assert_eq!(fresh().sync_state_hash(), (sum >> 64) as u64);
}
