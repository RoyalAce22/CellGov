//! An SPU get across the edge between two 64 KiB shared-memory segments
//! mapped end to end, as the `spu_get_shm_seam` capture shows it, run
//! through the runtime.
//!
//! On the console a 32-byte get that takes 16 bytes from each segment
//! completes with no exception, and the local store holds the bytes of
//! both segments in address order, as a get inside one segment does.

use cellgov_core::Runtime;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_GET, MFC_RD_TAG_STAT, MFC_TAG_UPDATE_ALL, MFC_WR_TAG_UPDATE,
};
use cellgov_ps3_abi::lv2::errno::CELL_ETIMEDOUT;
use cellgov_ps3_abi::lv2::spu::group_join_cause;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/spu_get_shm_seam/ps3/cech20-cex-493/observation.json";

/// The get's start below the edge per case: inside the low segment, then
/// across the edge.
const OFFSET: [u64; 2] = [48, 16];
/// The get's size.
const BYTES: usize = 32;
/// One segment.
const K64: u64 = 0x1_0000;
/// Where the get lands in local store.
const LSA: usize = 0x400;
/// A bound on the run's steps: the get, the tag wait and the stop.
const STEPS: usize = 64;

/// The capture as big-endian u32 words.
fn console() -> Vec<u32> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 192);
    p.chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4")))
        .collect()
}

/// The probe's fill of the 64 bytes from the edge - 48: 0x10, 0x11, ...
fn fill() -> Vec<u8> {
    (0x10..0x50).collect()
}

/// Runs a 32-byte get at `ea` on one SPU over the two segments at `area`
/// and returns what landed in local store.
fn run(area: u64, ea: u64) -> Vec<u8> {
    let edge = area + K64;
    let mut memory = GuestMemory::from_regions(vec![
        Region::new(0, 0x1000, "main", PageSize::Page64K),
        Region::new(area, K64 as usize, "m0", PageSize::Page64K),
        Region::new(edge, K64 as usize, "m1", PageSize::Page64K),
    ])
    .expect("the regions tile");
    let range = ByteRange::new(GuestAddr::new(edge - 48), 64).expect("range");
    memory.apply_commit(range, &fill()).expect("the fill");
    let mut rt = Runtime::new(memory, Budget::new(100), 400);
    let spu = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        let program = [
            (0x10D << 21) | (u32::from(MFC_CMD) << 7) | 2,
            (0x10D << 21) | (u32::from(MFC_WR_TAG_UPDATE) << 7) | 7,
            (0x00D << 21) | (u32::from(MFC_RD_TAG_STAT) << 7) | 5,
            0,
        ];
        for (i, word) in program.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        state.set_reg_word_splat(2, MFC_GET);
        state.set_reg_word_splat(7, MFC_TAG_UPDATE_ALL);
        let c = &mut state.channels;
        c.mfc_lsa = LSA as u32;
        c.mfc_eah = (ea >> 32) as u32;
        c.mfc_eal = ea as u32;
        c.mfc_size = BYTES as u32;
        c.mfc_tag_id = 1;
        c.tag_mask = 1 << 1;
        spu
    });
    for _ in 0..STEPS {
        let Ok(mut step) = rt.step() else { break };
        if rt.commit_step(&mut step).is_err() {
            break;
        }
    }
    assert!(rt.take_mfc_exception().is_none(), "the get raised nothing");
    let unit = rt.registry().get(spu).expect("the SPU").as_any();
    unit.downcast_ref::<SpuExecutionUnit>()
        .expect("an SPU unit")
        .state()
        .ls[LSA..LSA + BYTES]
        .to_vec()
}

#[test]
fn a_get_across_two_segments_end_to_end_lands_both_segments_bytes_as_on_the_console() {
    let w = console();
    assert_eq!(w[0..3], [0, 0, OFFSET.len() as u32], "no step failed");
    let area = u64::from(w[5]);
    for (k, &offset) in OFFSET.iter().enumerate() {
        let c = &w[16 + 16 * k..][..16];
        assert_eq!(c[0], CELL_ETIMEDOUT.code, "case {k}: no exception event");
        assert_eq!(
            c[3..5],
            [0, group_join_cause::ALL_THREADS_EXIT],
            "case {k}: the thread exited"
        );
        let landed: Vec<u8> = c[8..16].iter().flat_map(|v| v.to_be_bytes()).collect();
        assert_eq!(
            run(area, area + K64 - offset),
            landed,
            "case {k}: the get at the edge - {offset}"
        );
    }
}
