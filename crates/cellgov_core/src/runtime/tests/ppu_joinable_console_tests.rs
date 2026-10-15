//! A thread `_sys_ppu_thread_create` makes without the JOINABLE flag,
//! held to the `lv2_ppu_thread_joinable` capture from the reference
//! console: a join answers at once, and the thread's exit collects it.

use super::*;

/// The captured payload as big-endian words.
fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/lv2_ppu_thread_joinable/ps3/cech20-cex-493/cgov_frame.bin");
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
const VALUE_PTR: u64 = 0x300;
const PRIO_PTR: u64 = 0x310;

fn write(rt: &mut Runtime, addr: u64, bytes: &[u8]) {
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), bytes.len() as u64)
        .expect("a range");
    rt.memory_mut()
        .apply_commit(range, bytes)
        .expect("the write");
}

fn call(rt: &mut Runtime, unit: UnitId, number: u64, args: &[u64]) -> Option<u64> {
    let mut all = [0u64; 8];
    all[..args.len()].copy_from_slice(args);
    rt.dispatch_lv2_request(cellgov_lv2::request::classify(number, &all), unit);
    rt.registry_mut().drain_syscall_return(unit)
}

#[test]
fn a_thread_created_without_the_joinable_flag_answers_as_on_the_console() {
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
    write(&mut rt, VALUE_PTR, &[0xFF; 8]);

    let created = call(
        &mut rt,
        creator,
        52,
        &[ID_PTR, PARAM, 0, 0, 1000, 0x4000, 0, 0],
    );
    assert_eq!(created, Some(0));
    let thread_id = u64::from(read_guest_u32_be(&rt, ID_PTR as u32 + 4));
    let child = rt
        .lv2_host()
        .ppu_threads()
        .get(cellgov_lv2::ppu_thread::PpuThreadId::new(thread_id))
        .expect("the created thread")
        .unit_id;
    assert_eq!(call(&mut rt, creator, 53, &[thread_id, 0]), Some(0));

    let join = call(&mut rt, creator, 44, &[thread_id, VALUE_PTR]).expect("the join answers");
    assert_eq!(
        rt.registry().effective_status(creator),
        Some(cellgov_exec::UnitStatus::Runnable),
        "the join does not wait"
    );
    assert_eq!(
        [join as u32, read_guest_u32_be(&rt, VALUE_PTR as u32 + 4)],
        [console[5], console[6]],
        "w5, w6"
    );

    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::PpuThreadExit { exit_value: 0x0E },
        child,
    );
    let after = [
        call(&mut rt, creator, 44, &[thread_id, VALUE_PTR]),
        call(&mut rt, creator, 48, &[thread_id, PRIO_PTR]),
        call(&mut rt, creator, 53, &[thread_id, 0]),
    ]
    .map(|code| code.expect("the call answers") as u32);
    assert_eq!(
        after,
        [console[9], console[10], console[11]],
        "w9, w10, w11"
    );
}
