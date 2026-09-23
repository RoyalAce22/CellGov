//! Tests committed-memory read footprints across PPU execution paths.

use cellgov_fuzz::ppu_paths::{first_path_divergence, run_all_paths, PpuExecutionPath};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ppu::state::PpuState;

const DATA_BASE: u64 = 0x1000_0000;

fn lwz(rt: u32, ra: u32, offset: u16) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(offset)
}

fn stw(rs: u32, ra: u32, offset: u16) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(offset)
}

fn sth(rs: u32, ra: u32, offset: u16) -> u32 {
    (44 << 26) | (rs << 21) | (ra << 16) | u32::from(offset)
}

#[test]
fn ordinary_load_retains_the_same_data_read_on_every_path() {
    let mut state = PpuState::new();
    state.set_gpr(4, DATA_BASE);
    let runs = run_all_paths(&[lwz(3, 4, 0)], &state, &[0; 64]).expect("mapped load must run");
    let range = ByteRange::new(GuestAddr::new(DATA_BASE), 4).expect("fixed range must fit");

    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    assert!(runs.iter().all(|run| run.committed_data_reads == [range]));
}

#[test]
fn a_fully_forwarded_load_has_no_committed_memory_read() {
    let mut state = PpuState::new();
    state.set_gpr(3, 0x1234_5678);
    state.set_gpr(4, DATA_BASE);
    let runs = run_all_paths(&[stw(3, 4, 0), lwz(5, 4, 0)], &state, &[0; 64])
        .expect("forwarded load must run");

    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    assert!(runs.iter().all(|run| run.committed_data_reads.is_empty()));
}

#[test]
fn partial_overlap_still_reads_committed_memory() {
    let mut state = PpuState::new();
    state.set_gpr(3, 0x1234);
    state.set_gpr(4, DATA_BASE);
    let runs = run_all_paths(&[sth(3, 4, 0), lwz(5, 4, 0)], &state, &[0; 64])
        .expect("partial overlap must run");
    let range = ByteRange::new(GuestAddr::new(DATA_BASE), 4).expect("fixed range must fit");

    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    assert!(runs.iter().all(|run| run.committed_data_reads == [range]));
}

#[test]
fn a_missing_or_misattributed_data_read_names_the_path_pair() {
    let mut state = PpuState::new();
    state.set_gpr(4, DATA_BASE);
    let mut runs = run_all_paths(&[lwz(3, 4, 0)], &state, &[0; 64]).expect("mapped load must run");
    runs[1].committed_data_reads.clear();

    let divergence = first_path_divergence(&runs).expect("missing read must diverge");

    assert_eq!(divergence.left, PpuExecutionPath::Plain);
    assert_eq!(divergence.right, PpuExecutionPath::Forwarded);
    assert!(divergence.data_reads_differ);
    assert!(!divergence.stop_differs);
}
