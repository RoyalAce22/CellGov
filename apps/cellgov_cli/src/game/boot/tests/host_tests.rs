//! The cellSysutil slot-state shm seed.

use super::cellsysutil_system_seed;

#[test]
fn cellsysutil_seed_covers_both_slots_with_v256_ring() {
    let seed = cellsysutil_system_seed();
    assert_eq!(
        seed.shm_ipc_key,
        cellgov_ps3_abi::lv2::ipc::CELLSYSUTIL_SHM_IPC_KEY
    );
    for slot_base in [0u32, 0x8000] {
        let field = |off: u32| -> &[u8] {
            &seed
                .writes
                .iter()
                .find(|(o, _)| *o == slot_base + off)
                .unwrap_or_else(|| panic!("missing seed write at slot+{off:#x}"))
                .1
        };
        assert_eq!(field(0), 0x40u32.to_be_bytes());
        assert_eq!(field(4), 256u32.to_be_bytes(), "limit");
        // The six measured dispatcher field budgets drain inside the
        // seeded ring.
        let limit = u32::from_be_bytes(field(4).try_into().unwrap());
        assert!(56 + 8 + 76 + 4 + 22 + 10 <= limit);
        assert_eq!(field(8), 0u32.to_be_bytes(), "read_pos");
        assert_eq!(field(12), 256u32.to_be_bytes(), "write_pos");
        assert_eq!(field(16), 0u32.to_be_bytes(), "cursor");
        assert_eq!(field(20), 1u32.to_be_bytes(), "state");
        assert_eq!(field(30), [0u8], "predicate");
        let payload = field(0x40);
        assert_eq!(payload.len(), 256);
        assert!(payload.iter().all(|&b| b == 0));
    }
}

#[test]
fn cellsysutil_seed_writes_stay_inside_the_64k_shm() {
    let seed = cellsysutil_system_seed();
    for (offset, bytes) in &seed.writes {
        assert!(
            u64::from(*offset) + bytes.len() as u64 <= 0x10000,
            "seed write at +{offset:#x} ({} bytes) exceeds the 64 KiB shm",
            bytes.len(),
        );
    }
}
