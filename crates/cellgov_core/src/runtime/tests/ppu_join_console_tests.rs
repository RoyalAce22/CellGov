//! A PPU thread join with a NULL vptr, held to the
//! `lv2_ppu_thread_answers` capture from the reference console: the
//! join waits for the thread, collects it and answers CELL_EFAULT, and a
//! second join of the same thread answers as for an unknown id.

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

#[test]
fn a_join_with_a_null_vptr_collects_the_thread_as_on_the_console() {
    let console = console_words();
    assert_eq!((console[0], console[1]), (0, 0), "the probe ran every step");
    // The console's NULL-vptr join took timebase ticks: it waited for
    // the sleeping thread.
    assert!(console[28] > 0, "the console's join waited");

    let mut rt = build(0x1000, 1, 100);
    let joiner = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let exiter = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let attrs = || cellgov_lv2::PpuThreadAttrs {
        entry: 0x100,
        arg: 0,
        stack_base: 0,
        stack_size: 0,
        priority: 1000,
        tls_base: 0,
    };
    rt.lv2_host_mut()
        .ppu_threads_mut()
        .create(joiner, attrs())
        .expect("joiner thread");
    let target = rt
        .lv2_host_mut()
        .ppu_threads_mut()
        .create(exiter, attrs())
        .expect("exiter thread")
        .raw();
    const VALUE_PTR: u32 = 0x200;
    rt.memory_mut()
        .apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(u64::from(VALUE_PTR)), 8)
                .expect("a word"),
            &[0xFF; 8],
        )
        .expect("the pre-fill");

    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::PpuThreadJoin {
            target,
            status_out_ptr: 0,
        },
        joiner,
    );
    assert_eq!(
        rt.registry().effective_status(joiner),
        Some(cellgov_exec::UnitStatus::Blocked),
        "the join waits for the running thread"
    );
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::PpuThreadExit { exit_value: 0x1234 },
        exiter,
    );
    let first = rt
        .registry_mut()
        .drain_syscall_return(joiner)
        .expect("the join answers");

    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::PpuThreadJoin {
            target,
            status_out_ptr: VALUE_PTR,
        },
        joiner,
    );
    let second = rt
        .registry_mut()
        .drain_syscall_return(joiner)
        .expect("the second join answers");

    assert_eq!(
        [
            first as u32,
            second as u32,
            read_guest_u32_be(&rt, VALUE_PTR + 4)
        ],
        [console[27], console[29], console[30]],
        "w27, w29, w30"
    );
}
