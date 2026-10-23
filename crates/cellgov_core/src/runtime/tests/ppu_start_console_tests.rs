//! A thread `_sys_ppu_thread_create` made runs only once
//! `sys_ppu_thread_start` names it, held to the `lv2_ppu_thread_answers`
//! capture from the reference console: the probe's thread B wrote no
//! reached flag before the start and wrote it after.

use super::*;

/// The captured payload as big-endian words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/lv2_ppu_thread_answers/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

const PARAM: u64 = 0x100;
const OPD: u64 = 0x110;
const ID_PTR: u64 = 0x200;
/// Steps the creator takes while the created thread waits.
const WAIT_STEPS: usize = 6;

fn write(rt: &mut Runtime, addr: u64, bytes: &[u8]) {
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), bytes.len() as u64)
        .expect("a range");
    rt.memory_mut()
        .apply_commit(range, bytes)
        .expect("the write");
}

/// Run one step and name the unit it scheduled.
fn step(rt: &mut Runtime) -> UnitId {
    let step = rt.step().expect("a unit runs");
    rt.commit_unchecked(&step.result, &step.effects)
        .expect("the step commits");
    step.unit
}

#[test]
fn a_created_thread_runs_only_after_its_start_as_on_the_console() {
    let console = console_words();
    assert_eq!((console[0], console[1]), (0, 0), "the probe ran every step");

    let mut rt = build(0x1000, 1, 1000);
    let creator = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1000));
    rt.lv2_host_mut().ppu_threads_mut().insert_primary(
        creator,
        cellgov_lv2::PpuThreadAttrs {
            entry: 0x100,
            arg: 0,
            stack_base: 0,
            stack_size: 0,
            priority: 1000,
            tls_base: 0,
        },
    );
    rt.set_ppu_factory(|id, _init| Box::new(CountingUnit::new(id, 1000)));
    let mut param = [0u8; 8];
    param[0..4].copy_from_slice(&(OPD as u32).to_be_bytes());
    write(&mut rt, PARAM, &param);
    write(&mut rt, OPD, &[0, 0, 0x04, 0, 0, 0, 0x08, 0]);

    rt.dispatch_lv2_request(
        cellgov_lv2::request::classify(52, &[ID_PTR, PARAM, 0, 0, 1000, 0x4000, 1, 0]),
        creator,
    );
    assert_eq!(rt.registry_mut().drain_syscall_return(creator), Some(0));
    let thread_id = u64::from(read_guest_u32_be(&rt, ID_PTR as u32 + 4));
    let child = rt
        .lv2_host()
        .ppu_threads()
        .get(cellgov_lv2::ppu_thread::PpuThreadId::new(thread_id))
        .expect("the created thread")
        .unit_id;

    let ran_before_start = (0..WAIT_STEPS).any(|_| step(&mut rt) == child);
    assert_eq!(
        u32::from(ran_before_start),
        console[32],
        "w32: the thread ran before 53"
    );

    rt.dispatch_lv2_request(
        cellgov_lv2::request::classify(53, &[thread_id, 0, 0, 0, 0, 0, 0, 0]),
        creator,
    );
    let start = rt
        .registry_mut()
        .drain_syscall_return(creator)
        .expect("the start answers");
    let ran_after_start = (0..WAIT_STEPS).any(|_| step(&mut rt) == child);
    assert_eq!(
        [start as u32, u32::from(ran_after_start)],
        [console[33], console[34]],
        "w33, w34"
    );
}
