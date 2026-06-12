//! Syscall-to-Lv2Request classification tests, including arg-narrowing coverage tables and hypercall routing.

use super::*;

#[test]
fn classify_unresolved_import_pulls_nid_from_r4() {
    // Trampoline body loads NID into r4 (args[1]) and sets r11
    // to syscall::UNRESOLVED_IMPORT.
    let args = [0xdead_dead, 0x1234_5678, 0, 0, 0, 0, 0, 0];
    let req = classify(syscall::UNRESOLVED_IMPORT, &args);
    assert_eq!(req, Lv2Request::UnresolvedImport { nid: 0x1234_5678 });
}

#[test]
fn classify_unresolved_import_namespace_above_base_is_unsupported() {
    // A syscall in the UnresolvedImport namespace but not at
    // the reserved UNRESOLVED_IMPORT slot routes to Unsupported.
    // This keeps room for future per-slot trampolines that
    // encode the slot index in the syscall number.
    let args = [0; 8];
    let req = classify(syscall::UNRESOLVED_IMPORT + 1, &args);
    assert!(matches!(req, Lv2Request::Unsupported { .. }));
}

#[test]
fn classify_spu_image_open() {
    let args = [0x1000, 0x2000, 0, 0, 0, 0, 0, 0];
    let req = classify(156, &args);
    assert_eq!(
        req,
        Lv2Request::SpuImageOpen {
            img_ptr: 0x1000,
            path_ptr: 0x2000,
        }
    );
}

#[test]
fn classify_spu_image_import() {
    let args = [0x1000, 0x2000, 0x4000, 0xAA, 0, 0, 0, 0];
    let req = classify(158, &args);
    assert_eq!(
        req,
        Lv2Request::SpuImageImport {
            handle_out: 0x1000,
            img_ptr: 0x2000,
            size: 0x4000,
            type_id: 0xAA,
        }
    );
}

#[test]
fn classify_thread_group_create() {
    let args = [0x3000, 2, 100, 0x4000, 0, 0, 0, 0];
    let req = classify(170, &args);
    assert_eq!(
        req,
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x3000,
            num_threads: 2,
            priority: 100,
            attr_ptr: 0x4000,
        }
    );
}

#[test]
fn classify_thread_initialize() {
    let args = [0x6000, 1, 0, 0x7000, 0x8000, 0x9000, 0, 0];
    let req = classify(172, &args);
    assert_eq!(
        req,
        Lv2Request::SpuThreadInitialize {
            thread_ptr: 0x6000,
            group_id: 1,
            thread_num: 0,
            img_ptr: 0x7000,
            attr_ptr: 0x8000,
            arg_ptr: 0x9000,
        }
    );
}

#[test]
fn classify_thread_group_start() {
    let args = [7, 0, 0, 0, 0, 0, 0, 0];
    let req = classify(173, &args);
    assert_eq!(req, Lv2Request::SpuThreadGroupStart { group_id: 7 });
}

#[test]
fn classify_thread_group_join() {
    let args = [3, 0x6000, 0x7000, 0, 0, 0, 0, 0];
    let req = classify(178, &args);
    assert_eq!(
        req,
        Lv2Request::SpuThreadGroupJoin {
            group_id: 3,
            cause_ptr: 0x6000,
            status_ptr: 0x7000,
        }
    );
}

#[test]
fn classify_thread_group_terminate_is_separate_from_join() {
    let args = [3, 0xFFFF_FFFF_FFFF_FFFF, 0, 0, 0, 0, 0, 0];
    let req = classify(177, &args);
    assert_eq!(
        req,
        Lv2Request::SpuThreadGroupTerminate {
            group_id: 3,
            value: -1,
        }
    );
}

#[test]
fn classify_tty_write() {
    let args = [0, 0x8000, 64, 0x9000, 0, 0, 0, 0];
    let req = classify(403, &args);
    assert_eq!(
        req,
        Lv2Request::TtyWrite {
            fd: 0,
            buf_ptr: 0x8000,
            len: 64,
            nwritten_ptr: 0x9000,
        }
    );
}

#[test]
fn classify_process_exit_accepts_zero() {
    let args = [0, 0, 0, 0, 0, 0, 0, 0];
    let req = classify(syscall::PROCESS_EXIT, &args);
    assert_eq!(req, Lv2Request::ProcessExit { code: 0 });
}

#[test]
fn classify_process_exit_accepts_sign_extended_negative() {
    // Guest C `exit(-1)`: PPC64 sign-extends to 0xFFFF_FFFF_FFFF_FFFF.
    let args = [0xFFFF_FFFF_FFFF_FFFF, 0, 0, 0, 0, 0, 0, 0];
    let req = classify(syscall::PROCESS_EXIT, &args);
    assert_eq!(req, Lv2Request::ProcessExit { code: -1 });
}

#[test]
fn classify_process_exit_rejects_non_i32_range() {
    // 2^32 + 1: not a clean sign extension; cannot represent as i32.
    let args = [0x1_0000_0001, 0, 0, 0, 0, 0, 0, 0];
    match classify(syscall::PROCESS_EXIT, &args) {
        Lv2Request::Malformed { number, reason, .. } => {
            assert_eq!(number, syscall::PROCESS_EXIT);
            assert!(
                reason.contains("arg 0") && reason.contains("i32"),
                "unexpected reason: {reason}",
            );
        }
        other => panic!("expected Malformed, got {other:?}"),
    }
}

#[test]
fn classify_ppu_thread_yield() {
    let args = [0xDEAD, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(classify(43, &args), Lv2Request::PpuThreadYield);
}

#[test]
fn classify_time_get_timebase_frequency_ignores_args() {
    let args = [0xDEAD, 0xBEEF, 0, 0, 0, 0, 0, 0];
    assert_eq!(classify(147, &args), Lv2Request::TimeGetTimebaseFrequency);
}

#[test]
fn classify_time_get_current_time_captures_out_pointers() {
    let args = [0x9000, 0x9008, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(145, &args),
        Lv2Request::TimeGetCurrentTime {
            sec_ptr: 0x9000,
            nsec_ptr: 0x9008,
        }
    );
}

#[test]
fn classify_time_get_timezone_captures_out_pointers() {
    let args = [0xd000_fd10, 0xd000_fd14, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(144, &args),
        Lv2Request::TimeGetTimezone {
            timezone_ptr: 0xd000_fd10,
            summer_time_ptr: 0xd000_fd14,
        }
    );
}

#[test]
fn classify_memory_get_user_memory_size_captures_out_pointer() {
    let args = [0xd000_fdf4, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(352, &args),
        Lv2Request::MemoryGetUserMemorySize {
            mem_info_ptr: 0xd000_fdf4,
        }
    );
}

#[test]
fn classify_ppu_thread_exit_captures_exit_value() {
    let args = [0xDEAD_BEEF, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(41, &args),
        Lv2Request::PpuThreadExit {
            exit_value: 0xDEAD_BEEF
        },
    );
}

#[test]
fn classify_ppu_thread_join_captures_target_and_out_ptr() {
    let args = [0x0100_0003, 0x5000, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(44, &args),
        Lv2Request::PpuThreadJoin {
            target: 0x0100_0003,
            status_out_ptr: 0x5000,
        },
    );
}

#[test]
fn classify_ppu_thread_create_captures_all_fields() {
    let args = [0x3000, 0x2_0000, 0xCAFE_BABE, 1500, 0x10_000, 0, 0, 0];
    assert_eq!(
        classify(52, &args),
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x3000,
            param_ptr: 0x2_0000,
            arg: 0xCAFE_BABE,
            priority: 1500,
            stacksize: 0x10_000,
            flags: 0,
        },
    );
}

#[test]
fn classify_unknown_syscall() {
    let args = [0xAA, 0xBB, 0xCC, 0, 0, 0, 0, 0];
    let req = classify(999, &args);
    assert_eq!(
        req,
        Lv2Request::Unsupported {
            number: 999,
            args: [0xAA, 0xBB, 0xCC, 0, 0, 0, 0, 0],
        }
    );
}

#[test]
fn unsupported_preserves_args_for_diagnosis() {
    let args = [0x1234, 0x5678, 0x9ABC, 0xDEF0, 1, 2, 3, 4];
    let req = classify(600, &args);
    match req {
        Lv2Request::Unsupported { number, args: a } => {
            assert_eq!(number, 600);
            assert_eq!(a, args);
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn spu_thread_group_range_stubs_classify_as_unsupported() {
    let args = [0; 8];
    for n in [174, 175, 176, 179, 180, 192] {
        let req = classify(n, &args);
        assert!(
            matches!(req, Lv2Request::Unsupported { number, .. } if number == n),
            "syscall {n} should be Unsupported",
        );
    }
}

#[test]
fn classify_mutex_create() {
    let args = [0x5000, 0x6000, 0, 0, 0, 0, 0, 0];
    let req = classify(100, &args);
    assert_eq!(
        req,
        Lv2Request::MutexCreate {
            id_ptr: 0x5000,
            attr_ptr: 0x6000,
        }
    );
}

#[test]
fn classify_lwmutex_create_destroy() {
    let create_args = [0x5000, 0x6000, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(95, &create_args),
        Lv2Request::LwMutexCreate {
            id_ptr: 0x5000,
            attr_ptr: 0x6000,
        }
    );
    let destroy_args = [7, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(96, &destroy_args),
        Lv2Request::LwMutexDestroy { id: 7 }
    );
}

#[test]
fn classify_lwmutex_lock() {
    let args = [7, 100, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(97, &args),
        Lv2Request::LwMutexLock {
            id: 7,
            mutex_ptr: 0,
            timeout: 100
        }
    );
}

#[test]
fn classify_lwmutex_unlock() {
    let args = [9, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(classify(98, &args), Lv2Request::LwMutexUnlock { id: 9 });
}

#[test]
fn classify_lwmutex_trylock() {
    let args = [11, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(classify(99, &args), Lv2Request::LwMutexTryLock { id: 11 });
}

#[test]
fn classify_mutex_trylock() {
    let args = [42, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(103, &args),
        Lv2Request::MutexTryLock { mutex_id: 42 }
    );
}

#[test]
fn classify_event_queue_receive_send() {
    assert_eq!(
        classify(130, &[7, 0x1000, 500, 0, 0, 0, 0, 0]),
        Lv2Request::EventQueueReceive {
            queue_id: 7,
            out_ptr: 0x1000,
            timeout: 500,
        }
    );
    assert_eq!(
        classify(138, &[7, 0xaa, 0xbb, 0xcc, 0, 0, 0, 0]),
        Lv2Request::EventPortSend {
            port_id: 7,
            data1: 0xaa,
            data2: 0xbb,
            data3: 0xcc,
        }
    );
    assert_eq!(
        classify(131, &[7, 0x2000, 4, 0x3000, 0, 0, 0, 0]),
        Lv2Request::EventQueueTryReceive {
            queue_id: 7,
            event_array: 0x2000,
            size: 4,
            count_out: 0x3000,
        }
    );
}

#[test]
fn classify_semaphore_trywait_and_get_value() {
    assert_eq!(
        classify(93, &[7, 0, 0, 0, 0, 0, 0, 0]),
        Lv2Request::SemaphoreTryWait { id: 7 }
    );
    assert_eq!(
        classify(114, &[7, 0x1000, 0, 0, 0, 0, 0, 0]),
        Lv2Request::SemaphoreGetValue {
            id: 7,
            out_ptr: 0x1000
        }
    );
}

#[test]
fn classify_semaphore_post() {
    let args = [7, 1, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(94, &args),
        Lv2Request::SemaphorePost { id: 7, val: 1 }
    );
}

#[test]
fn classify_semaphore_create_destroy_wait() {
    let create_args = [0x5000, 0x6000, 2, 10, 0, 0, 0, 0];
    assert_eq!(
        classify(90, &create_args),
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x5000,
            attr_ptr: 0x6000,
            initial: 2,
            max: 10,
        }
    );
    let destroy_args = [7, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(91, &destroy_args),
        Lv2Request::SemaphoreDestroy { id: 7 }
    );
    let wait_args = [7, 100, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(92, &wait_args),
        Lv2Request::SemaphoreWait {
            id: 7,
            timeout: 100
        }
    );
}

#[test]
fn classify_mutex_lock_unlock() {
    let args = [42, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(102, &args),
        Lv2Request::MutexLock {
            mutex_id: 42,
            timeout: 0,
        }
    );
    assert_eq!(
        classify(104, &args),
        Lv2Request::MutexUnlock { mutex_id: 42 }
    );
}

#[test]
fn classify_event_queue_create_destroy() {
    let args = [0x7000, 0x8000, 0x100, 64, 0, 0, 0, 0];
    assert_eq!(
        classify(128, &args),
        Lv2Request::EventQueueCreate {
            id_ptr: 0x7000,
            attr_ptr: 0x8000,
            key: 0x100,
            size: 64,
        }
    );
    let args2 = [99, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(129, &args2),
        Lv2Request::EventQueueDestroy { queue_id: 99 }
    );
}

#[test]
fn classify_cond_create_destroy_wait() {
    assert_eq!(
        classify(105, &[0x5000, 7, 0x6000, 0, 0, 0, 0, 0]),
        Lv2Request::CondCreate {
            id_ptr: 0x5000,
            mutex_id: 7,
            attr_ptr: 0x6000,
        }
    );
    assert_eq!(
        classify(106, &[9, 0, 0, 0, 0, 0, 0, 0]),
        Lv2Request::CondDestroy { id: 9 }
    );
    assert_eq!(
        classify(107, &[9, 500, 0, 0, 0, 0, 0, 0]),
        Lv2Request::CondWait {
            id: 9,
            timeout: 500,
        }
    );
}

#[test]
fn classify_cond_signal_variants() {
    assert_eq!(
        classify(108, &[9, 0, 0, 0, 0, 0, 0, 0]),
        Lv2Request::CondSignal { id: 9 }
    );
    assert_eq!(
        classify(109, &[9, 0, 0, 0, 0, 0, 0, 0]),
        Lv2Request::CondSignalAll { id: 9 }
    );
    assert_eq!(
        classify(110, &[9, 0x0100_0005, 0, 0, 0, 0, 0, 0]),
        Lv2Request::CondSignalTo {
            id: 9,
            target_thread: 0x0100_0005,
        }
    );
}

#[test]
fn classify_memory_allocate_free() {
    let args = [0x10000, 0x200, 0x9000, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(348, &args),
        Lv2Request::MemoryAllocate {
            size: 0x10000,
            flags: 0x200,
            alloc_addr_ptr: 0x9000,
        }
    );
    let args2 = [0x0001_0000, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(349, &args2),
        Lv2Request::MemoryFree { addr: 0x0001_0000 }
    );
}

#[test]
fn narrow_ptr_rejects_high_bits_in_u32_field() {
    let args = [0x1_0000_1000, 0x2000, 0, 0, 0, 0, 0, 0];
    match classify(100, &args) {
        Lv2Request::Malformed {
            number,
            reason,
            args: a,
        } => {
            assert_eq!(number, 100);
            assert!(
                reason.contains("arg 0") && reason.contains("high"),
                "unexpected reason: {reason}",
            );
            assert_eq!(a, args);
        }
        other => panic!("expected Malformed, got {other:?}"),
    }
}

#[test]
fn narrow_i32_accepts_sign_extended_negatives() {
    let args = [
        0x5000,
        0x6000,
        0xFFFF_FFFF_FFFF_FFFF,
        0xFFFF_FFFF_FFFF_FFFE,
        0,
        0,
        0,
        0,
    ];
    assert_eq!(
        classify(90, &args),
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x5000,
            attr_ptr: 0x6000,
            initial: -1,
            max: -2,
        }
    );
}

#[test]
fn narrow_i32_rejects_values_outside_i32_range() {
    let args = [0x5000, 0x6000, 0x1_0000_0001, 10, 0, 0, 0, 0];
    match classify(90, &args) {
        Lv2Request::Malformed { number, reason, .. } => {
            assert_eq!(number, 90);
            assert!(
                reason.contains("arg 2") && reason.contains("i32"),
                "unexpected reason: {reason}",
            );
        }
        other => panic!("expected Malformed, got {other:?}"),
    }
}

#[test]
fn narrow_i32_rejects_large_positive() {
    // 2^31 fits u32 but not i32; an `as i32` cast wraps to i32::MIN.
    let args = [0x5000, 0x6000, 0x8000_0000, 10, 0, 0, 0, 0];
    assert!(matches!(
        classify(90, &args),
        Lv2Request::Malformed { number: 90, .. }
    ));
}

/// Every (syscall, slot) pair listed here must reject a non-zero
/// high half rather than wrap. Coverage and the match's `p!()`
/// sites are kept in sync by `declared_narrowing_matches_classifier_behavior`
/// and `every_lv2_syscall_with_narrowing_appears_in_a_table`.
const U32_SLOTS_BY_SYSCALL: &[(u64, &[usize])] = &[
    (syscall::PROCESS_GETPID, &[]),
    (syscall::PROCESS_GET_NUMBER_OF_OBJECT, &[0, 1]),
    (syscall::PROCESS_GETPPID, &[]),
    (syscall::PROCESS_GET_SDK_VERSION, &[0, 1]),
    (syscall::PROCESS_GET_PARAMSFO, &[0]),
    (syscall::PROCESS_GET_PPU_GUID, &[]),
    (syscall::PROCESS_IS_SPU_LOCK_LINE_RESERVATION_ADDRESS, &[0]),
    (syscall::SPU_INITIALIZE, &[0, 1]),
    (syscall::TIMER_CREATE, &[0]),
    (syscall::TIMER_DESTROY, &[0]),
    (syscall::RWLOCK_CREATE, &[0, 1]),
    (syscall::RWLOCK_DESTROY, &[0]),
    (syscall::EVENT_PORT_CREATE, &[0, 1]),
    (syscall::EVENT_PORT_DESTROY, &[0]),
    (syscall::PPU_THREAD_JOIN, &[1]),
    (syscall::PPU_THREAD_CREATE, &[0, 1]),
    (syscall::EVENT_FLAG_CREATE, &[0, 1]),
    (syscall::EVENT_FLAG_DESTROY, &[0]),
    (syscall::EVENT_FLAG_WAIT, &[0, 2, 3]),
    (syscall::EVENT_FLAG_TRY_WAIT, &[0, 2, 3]),
    (syscall::EVENT_FLAG_SET, &[0]),
    (syscall::SEMAPHORE_CREATE, &[0, 1]),
    (syscall::SEMAPHORE_DESTROY, &[0]),
    (syscall::SEMAPHORE_WAIT, &[0]),
    (syscall::SEMAPHORE_TRY_WAIT, &[0]),
    (syscall::SEMAPHORE_POST, &[0]),
    (syscall::LWMUTEX_CREATE, &[0, 1]),
    (syscall::LWMUTEX_DESTROY, &[0]),
    (syscall::LWMUTEX_LOCK, &[0]),
    (syscall::LWMUTEX_UNLOCK, &[0]),
    (syscall::LWMUTEX_TRYLOCK, &[0]),
    (syscall::MUTEX_CREATE, &[0, 1]),
    (syscall::MUTEX_DESTROY, &[0]),
    (syscall::MUTEX_LOCK, &[0]),
    (syscall::MUTEX_UNLOCK, &[0]),
    (syscall::MUTEX_TRYLOCK, &[0]),
    (syscall::COND_CREATE, &[0, 1, 2]),
    (syscall::COND_DESTROY, &[0]),
    (syscall::COND_WAIT, &[0]),
    (syscall::COND_SIGNAL, &[0]),
    (syscall::COND_SIGNAL_ALL, &[0]),
    (syscall::COND_SIGNAL_TO, &[0, 1]),
    (syscall::SEMAPHORE_GET_VALUE, &[0, 1]),
    (syscall::EVENT_FLAG_CLEAR, &[0]),
    (syscall::EVENT_QUEUE_CREATE, &[0, 1, 3]),
    (syscall::EVENT_QUEUE_DESTROY, &[0]),
    (syscall::EVENT_QUEUE_RECEIVE, &[0, 1]),
    (syscall::EVENT_QUEUE_TRY_RECEIVE, &[0, 1, 2, 3]),
    (syscall::EVENT_FLAG_CANCEL, &[0, 1]),
    (syscall::EVENT_PORT_SEND, &[0]),
    (syscall::EVENT_FLAG_GET, &[0, 1]),
    (syscall::TIME_GET_TIMEZONE, &[0, 1]),
    (syscall::TIME_GET_CURRENT_TIME, &[0, 1]),
    (syscall::SPU_IMAGE_OPEN, &[0, 1]),
    (syscall::SPU_IMAGE_IMPORT, &[0, 1, 3]),
    (syscall::SPU_THREAD_GROUP_CREATE, &[0, 1, 3]),
    (syscall::SPU_THREAD_INITIALIZE, &[0, 1, 2, 3, 4, 5]),
    (syscall::SPU_THREAD_GROUP_START, &[0]),
    (syscall::SPU_THREAD_GROUP_DESTROY, &[0]),
    (syscall::SPU_THREAD_GROUP_TERMINATE, &[0]),
    (syscall::SPU_THREAD_GROUP_JOIN, &[0, 1, 2]),
    (syscall::SPU_THREAD_WRITE_MB, &[0, 1]),
    (syscall::MEMORY_CONTAINER_CREATE, &[0]),
    (syscall::MEMORY_ALLOCATE, &[2]),
    (syscall::MEMORY_FREE, &[0]),
    (syscall::MEMORY_GET_USER_MEMORY_SIZE, &[0]),
    (syscall::TTY_WRITE, &[0, 1, 2, 3]),
    (syscall::FS_OPEN, &[0, 1, 2, 3]),
    (syscall::FS_READ, &[0, 1, 3]),
    (syscall::FS_WRITE, &[0, 1, 3]),
    (syscall::FS_CLOSE, &[0]),
    (syscall::FS_OPENDIR, &[0, 1]),
    (syscall::FS_READDIR, &[0, 1, 2]),
    (syscall::FS_CLOSEDIR, &[0]),
    (syscall::FS_STAT, &[0, 1]),
    (syscall::FS_FSTAT, &[0, 1]),
    (syscall::FS_LSEEK, &[0, 2, 3]),
    (syscall::SYS_RSX_MEMORY_ALLOCATE, &[0, 1, 2]),
    (syscall::SYS_RSX_MEMORY_FREE, &[0]),
    (syscall::SYS_RSX_CONTEXT_ALLOCATE, &[0, 1, 2, 3]),
    (syscall::SYS_RSX_CONTEXT_FREE, &[0]),
    (syscall::SYS_RSX_CONTEXT_ATTRIBUTE, &[0, 1]),
    (syscall::SYS_RSX_CONTEXT_IOMAP, &[0, 1, 2, 3]),
    (syscall::SYS_RSX_DEVICE_MAP, &[0, 1, 2]),
];

#[test]
fn every_u32_slot_rejects_high_bits() {
    for &(num, slots) in U32_SLOTS_BY_SYSCALL {
        for &slot in slots {
            let mut args = [0u64; 8];
            args[slot] = 0x1_0000_0000;
            match classify(num, &args) {
                Lv2Request::Malformed {
                    number,
                    reason,
                    args: a,
                } => {
                    assert_eq!(number, num, "syscall {num} slot {slot}");
                    assert_eq!(a, args, "syscall {num} slot {slot}");
                    let tag = format!("arg {slot}");
                    assert!(
                        reason.contains(&tag),
                        "syscall {num} slot {slot}: reason {reason:?} did not name {tag:?}",
                    );
                }
                other => {
                    panic!("syscall {num} slot {slot}: expected Malformed, got {other:?}",)
                }
            }
        }
    }
}

#[test]
fn semaphore_post_val_narrowing() {
    let ok = [7u64, 0xFFFF_FFFF_FFFF_FFFF, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(94, &ok),
        Lv2Request::SemaphorePost { id: 7, val: -1 }
    );
    let bad = [7u64, 0x1_0000_0001, 0, 0, 0, 0, 0, 0];
    assert!(matches!(
        classify(94, &bad),
        Lv2Request::Malformed { number: 94, .. }
    ));
}

#[test]
fn classify_sys_rsx_memory_allocate() {
    let args = [0x1000, 0x1008, 0x0010_0000, 0x400, 0, 0, 0, 0];
    assert_eq!(
        classify(668, &args),
        Lv2Request::SysRsxMemoryAllocate {
            mem_handle_ptr: 0x1000,
            mem_addr_ptr: 0x1008,
            size: 0x0010_0000,
            flags: 0x400,
            a5: 0,
            a6: 0,
            a7: 0,
        }
    );
}

#[test]
fn classify_sys_rsx_memory_free() {
    let args = [0xA001, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(669, &args),
        Lv2Request::SysRsxMemoryFree { mem_handle: 0xA001 }
    );
}

#[test]
fn classify_sys_rsx_context_allocate() {
    let args = [0x2000, 0x2008, 0x2010, 0x2018, 0xA001, 0, 0, 0];
    assert_eq!(
        classify(670, &args),
        Lv2Request::SysRsxContextAllocate {
            context_id_ptr: 0x2000,
            lpar_dma_control_ptr: 0x2008,
            lpar_driver_info_ptr: 0x2010,
            lpar_reports_ptr: 0x2018,
            mem_ctx: 0xA001,
            system_mode: 0,
        }
    );
}

#[test]
fn classify_sys_rsx_context_free() {
    let args = [0x5555_5555, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(671, &args),
        Lv2Request::SysRsxContextFree {
            context_id: 0x5555_5555,
        }
    );
}

#[test]
fn real_lv2_syscalls_classify_into_lv2_namespace() {
    for n in [
        syscall::PROCESS_EXIT,
        syscall::PPU_THREAD_CREATE,
        syscall::FS_OPEN,
        syscall::TTY_WRITE,
    ] {
        assert_eq!(
            SyscallNamespace::of(n),
            Some(SyscallNamespace::Lv2),
            "syscall {n:#x} must classify into Lv2",
        );
    }
}

#[test]
fn classify_process_is_spu_lock_line_reservation_address() {
    let args = [0xE001_0000, 0x3, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::PROCESS_IS_SPU_LOCK_LINE_RESERVATION_ADDRESS, &args),
        Lv2Request::ProcessIsSpuLockLineReservationAddress {
            addr: 0xE001_0000,
            flags: 0x3,
        }
    );
}

#[test]
fn classify_spu_thread_group_destroy() {
    let args = [0x1234, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::SPU_THREAD_GROUP_DESTROY, &args),
        Lv2Request::SpuThreadGroupDestroy { id: 0x1234 }
    );
}

#[test]
fn classify_spu_initialize() {
    let args = [6, 1, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::SPU_INITIALIZE, &args),
        Lv2Request::SpuInitialize {
            max_usable_spu: 6,
            max_raw_spu: 1,
        }
    );
}

#[test]
fn classify_sys_rsx_context_attribute() {
    let args = [0x5555_5555, 0x102, 0xAA, 0xBB, 0xCC, 0xDD, 0, 0];
    assert_eq!(
        classify(674, &args),
        Lv2Request::SysRsxContextAttribute {
            context_id: 0x5555_5555,
            package_id: 0x102,
            a3: 0xAA,
            a4: 0xBB,
            a5: 0xCC,
            a6: 0xDD,
        }
    );
}

/// Every (syscall, slot) pair listed here must reject values
/// outside the i32 range. Coverage and the match's `s!()` sites
/// are kept in sync by the same cross-check tests that gate
/// [`U32_SLOTS_BY_SYSCALL`].
const S32_SLOTS_BY_SYSCALL: &[(u64, &[usize])] = &[
    (syscall::PROCESS_EXIT, &[0]),
    (syscall::PPU_THREAD_CREATE, &[3]),
    (syscall::SEMAPHORE_CREATE, &[2, 3]),
    (syscall::SEMAPHORE_POST, &[1]),
    (syscall::SPU_THREAD_GROUP_CREATE, &[2]),
    (syscall::SPU_THREAD_GROUP_TERMINATE, &[1]),
];

/// Probe slot `slot` of `num`'s classification with two values
/// that distinguish all three narrowing kinds, and return the
/// per-slot rejection profile.
///
/// | Probe                   | `p!()` rejects | `s!()` rejects | raw / literal rejects |
/// | ----------------------- | -------------- | -------------- | --------------------- |
/// | `0xFFFF_FFFF_FFFF_FFFF` | yes            | no (-1 fits)   | no                    |
/// | `0x1_0000_0000`         | yes            | yes (>i32 max) | no                    |
///
/// `p!()` and `s!()` short-circuit via `return`, so probing one
/// slot in isolation isolates that slot's behaviour regardless
/// of struct-field evaluation order.
fn derive_narrowing(num: u64) -> (Vec<usize>, Vec<usize>) {
    let mut p = vec![];
    let mut s = vec![];
    for slot in 0..8usize {
        let tag = format!("arg {slot}");
        let is_malformed_at_slot = |args: &[u64; 8]| {
            matches!(
                classify(num, args),
                Lv2Request::Malformed { reason, .. } if reason.contains(&tag)
            )
        };
        let mut a = [0u64; 8];
        a[slot] = 0xFFFF_FFFF_FFFF_FFFF;
        let rejects_minus_one = is_malformed_at_slot(&a);
        a[slot] = 0x1_0000_0000;
        let rejects_above_u32 = is_malformed_at_slot(&a);
        match (rejects_minus_one, rejects_above_u32) {
            (true, _) => p.push(slot),
            (false, true) => s.push(slot),
            (false, false) => {}
        }
    }
    (p, s)
}

#[test]
fn declared_narrowing_matches_classifier_behavior() {
    use std::collections::BTreeMap;
    let mut doc: BTreeMap<u64, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for &(n, slots) in U32_SLOTS_BY_SYSCALL {
        doc.entry(n).or_default().0.extend(slots.iter().copied());
    }
    for &(n, slots) in S32_SLOTS_BY_SYSCALL {
        doc.entry(n).or_default().1.extend(slots.iter().copied());
    }
    for (&num, (dp, ds)) in &doc {
        let (ep, es) = derive_narrowing(num);
        assert_eq!(
            *dp, ep,
            "syscall {num}: U32_SLOTS declared {dp:?}, classifier rejects at {ep:?}",
        );
        assert_eq!(
            *ds, es,
            "syscall {num}: S32_SLOTS declared {ds:?}, classifier rejects at {es:?}",
        );
    }
}

#[test]
fn every_lv2_syscall_with_narrowing_appears_in_a_table() {
    use std::collections::BTreeSet;
    let documented: BTreeSet<u64> = U32_SLOTS_BY_SYSCALL
        .iter()
        .chain(S32_SLOTS_BY_SYSCALL.iter())
        .map(|&(n, _)| n)
        .collect();
    for &num in syscall::ALL_LV2_NUMBERS {
        if documented.contains(&num) {
            continue;
        }
        let (p, s) = derive_narrowing(num);
        assert!(
            p.is_empty() && s.is_empty(),
            "syscall {num}: classifier narrows at p={p:?} s={s:?} but no table entry",
        );
    }
}

#[test]
fn every_s32_slot_rejects_out_of_range() {
    // 0x1_0000_0000: positive, doesn't fit i32 and fails the
    // i64-as-i32 narrowing. 0x8000_0000: fits u32 but is 2^31,
    // outside i32's positive range; `as i32` would wrap to
    // i32::MIN. The s! macro uses `i32::try_from` and rejects
    // both.
    for &probe in &[0x1_0000_0000u64, 0x8000_0000u64] {
        for &(num, slots) in S32_SLOTS_BY_SYSCALL {
            for &slot in slots {
                let mut args = [0u64; 8];
                args[slot] = probe;
                match classify(num, &args) {
                    Lv2Request::Malformed {
                        number,
                        reason,
                        args: a,
                    } => {
                        assert_eq!(number, num, "syscall {num} slot {slot} probe {probe:#x}");
                        assert_eq!(a, args, "syscall {num} slot {slot} probe {probe:#x}");
                        let tag = format!("arg {slot}");
                        assert!(
                            reason.contains(&tag) && reason.contains("i32"),
                            "syscall {num} slot {slot} probe {probe:#x}: reason {reason:?} did not name {tag:?} or i32",
                        );
                    }
                    other => panic!(
                        "syscall {num} slot {slot} probe {probe:#x}: expected Malformed, got {other:?}",
                    ),
                }
            }
        }
    }
}

#[test]
fn classify_with_lev_nonzero_routes_to_hypercall_skipping_lv2() {
    // Pick a syscall number that *would* match an LV2 arm
    // (PROCESS_EXIT) to prove the lev != 0 early return wins.
    let args = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22];
    let req = classify_with_lev(1, syscall::PROCESS_EXIT, &args);
    assert_eq!(
        req,
        Lv2Request::Hypercall {
            lev: NonZeroU8::new(1).unwrap(),
            r11: syscall::PROCESS_EXIT,
            args,
        }
    );
}

#[test]
fn classify_with_lev_hypercall_preserves_high_bits_without_narrowing() {
    // Args that would trip p!() on the LV2 path must pass through
    // the hypercall path verbatim.
    let args = [
        0x1_0000_0001,
        0xFFFF_FFFF_FFFF_FFFF,
        0x8000_0000,
        0,
        0,
        0,
        0,
        0,
    ];
    let req = classify_with_lev(7, 12345, &args);
    assert_eq!(
        req,
        Lv2Request::Hypercall {
            lev: NonZeroU8::new(7).unwrap(),
            r11: 12345,
            args,
        }
    );
}

#[test]
fn classify_with_lev_full_u8_range_routes_to_hypercall() {
    let args = [0; 8];
    let req = classify_with_lev(u8::MAX, syscall::PROCESS_EXIT, &args);
    assert!(matches!(
        req,
        Lv2Request::Hypercall {
            lev,
            r11: n,
            ..
        } if lev.get() == 255 && n == syscall::PROCESS_EXIT,
    ));
}

#[test]
fn classify_process_getpid_ignores_args() {
    let args = [0xDEAD, 0xBEEF, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::PROCESS_GETPID, &args),
        Lv2Request::ProcessGetPid
    );
}

#[test]
fn classify_process_get_number_of_object() {
    let args = [0x07, 0x9000, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::PROCESS_GET_NUMBER_OF_OBJECT, &args),
        Lv2Request::ProcessGetNumberOfObject {
            class_id: 0x07,
            count_out_ptr: 0x9000,
        }
    );
}

#[test]
fn classify_process_getppid_ignores_args() {
    let args = [0xDEAD, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::PROCESS_GETPPID, &args),
        Lv2Request::ProcessGetPpid
    );
}

#[test]
fn classify_process_get_sdk_version() {
    let args = [0x1234, 0x5000, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::PROCESS_GET_SDK_VERSION, &args),
        Lv2Request::ProcessGetSdkVersion {
            pid: 0x1234,
            version_out_ptr: 0x5000,
        }
    );
}

#[test]
fn classify_process_get_paramsfo() {
    let args = [0xA000, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::PROCESS_GET_PARAMSFO, &args),
        Lv2Request::ProcessGetParamsfo { buf_ptr: 0xA000 }
    );
}

#[test]
fn classify_process_get_ppu_guid_ignores_args() {
    let args = [0xDEAD, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::PROCESS_GET_PPU_GUID, &args),
        Lv2Request::ProcessGetPpuGuid
    );
}

#[test]
fn classify_timer_create_destroy() {
    assert_eq!(
        classify(syscall::TIMER_CREATE, &[0x5000, 0, 0, 0, 0, 0, 0, 0]),
        Lv2Request::TimerCreate { id_ptr: 0x5000 }
    );
    assert_eq!(
        classify(syscall::TIMER_DESTROY, &[9, 0, 0, 0, 0, 0, 0, 0]),
        Lv2Request::TimerDestroy { id: 9 }
    );
}

#[test]
fn classify_rwlock_create_destroy() {
    assert_eq!(
        classify(syscall::RWLOCK_CREATE, &[0x5000, 0x6000, 0, 0, 0, 0, 0, 0]),
        Lv2Request::RwlockCreate {
            id_ptr: 0x5000,
            attr_ptr: 0x6000,
        }
    );
    assert_eq!(
        classify(syscall::RWLOCK_DESTROY, &[11, 0, 0, 0, 0, 0, 0, 0]),
        Lv2Request::RwlockDestroy { id: 11 }
    );
}

#[test]
fn classify_event_port_create_destroy() {
    let args = [0x7000, 1, 0xCAFE_BABE_DEAD_BEEF, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::EVENT_PORT_CREATE, &args),
        Lv2Request::EventPortCreate {
            id_ptr: 0x7000,
            port_type: 1,
            name: 0xCAFE_BABE_DEAD_BEEF,
        }
    );
    assert_eq!(
        classify(syscall::EVENT_PORT_DESTROY, &[13, 0, 0, 0, 0, 0, 0, 0]),
        Lv2Request::EventPortDestroy { id: 13 }
    );
}

#[test]
fn classify_spu_thread_write_mb() {
    let args = [0x4242, 0xDEAD_BEEF, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::SPU_THREAD_WRITE_MB, &args),
        Lv2Request::SpuThreadWriteMb {
            thread_id: 0x4242,
            value: 0xDEAD_BEEF,
        }
    );
}

#[test]
fn classify_memory_container_create() {
    let args = [0x5000, 0x0040_0000, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::MEMORY_CONTAINER_CREATE, &args),
        Lv2Request::MemoryContainerCreate {
            cid_ptr: 0x5000,
            size: 0x0040_0000,
        }
    );
}

#[test]
fn classify_ss_access_control_engine() {
    let args = [2, 0x0100_0500, 0xd000_7d90, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::SS_ACCESS_CONTROL_ENGINE, &args),
        Lv2Request::SsAccessControlEngine {
            pkg_id: 2,
            a2: 0x0100_0500,
            a3: 0xd000_7d90,
        }
    );
}

#[test]
fn classify_fs_open() {
    let args = [0xA000, 0x01, 0xB000, 0o644, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::FS_OPEN, &args),
        Lv2Request::FsOpen {
            path_ptr: 0xA000,
            flags: 0x01,
            fd_out_ptr: 0xB000,
            mode: 0o644,
        }
    );
}

#[test]
fn classify_fs_read_preserves_u64_nbytes() {
    let args = [3, 0xB000, 0x1_0000_0000, 0xC000, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::FS_READ, &args),
        Lv2Request::FsRead {
            fd: 3,
            buf_ptr: 0xB000,
            nbytes: 0x1_0000_0000,
            nread_out_ptr: 0xC000,
        }
    );
}

#[test]
fn classify_fs_write_preserves_u64_size() {
    // sys_fs_write's size is uint64_t; a >=4 GiB value must
    // reach the variant intact rather than trip Malformed.
    let args = [3, 0xB000, 0x1_0000_0000, 0xC000, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::FS_WRITE, &args),
        Lv2Request::FsWrite {
            fd: 3,
            buf_ptr: 0xB000,
            size: 0x1_0000_0000,
            nwrite_ptr: 0xC000,
        }
    );
}

#[test]
fn classify_fs_close() {
    let args = [7, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::FS_CLOSE, &args),
        Lv2Request::FsClose { fd: 7 }
    );
}

#[test]
fn classify_fs_lseek_preserves_signed_offset() {
    // PPC64 sign extension: -1024 arrives as 0xFFFF_FFFF_FFFF_FC00.
    let args = [3, 0xFFFF_FFFF_FFFF_FC00, 1, 0xC000, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::FS_LSEEK, &args),
        Lv2Request::FsLseek {
            fd: 3,
            offset: -1024,
            whence: 1,
            pos_out_ptr: 0xC000,
        }
    );
}

#[test]
fn classify_fs_fstat_stat() {
    assert_eq!(
        classify(syscall::FS_FSTAT, &[3, 0xC000, 0, 0, 0, 0, 0, 0]),
        Lv2Request::FsFstat {
            fd: 3,
            stat_out_ptr: 0xC000,
        }
    );
    assert_eq!(
        classify(syscall::FS_STAT, &[0xA000, 0xC000, 0, 0, 0, 0, 0, 0]),
        Lv2Request::FsStat {
            path_ptr: 0xA000,
            stat_out_ptr: 0xC000,
        }
    );
}

#[test]
fn classify_fs_opendir_readdir_closedir() {
    assert_eq!(
        classify(syscall::FS_OPENDIR, &[0xA000, 0xC000, 0, 0, 0, 0, 0, 0]),
        Lv2Request::FsOpendir {
            path_ptr: 0xA000,
            fd_out_ptr: 0xC000,
        }
    );
    assert_eq!(
        classify(syscall::FS_READDIR, &[3, 0xB000, 0xC000, 0, 0, 0, 0, 0]),
        Lv2Request::FsReaddir {
            fd: 3,
            dirent_out_ptr: 0xB000,
            nread_out_ptr: 0xC000,
        }
    );
    assert_eq!(
        classify(syscall::FS_CLOSEDIR, &[3, 0, 0, 0, 0, 0, 0, 0]),
        Lv2Request::FsClosedir { fd: 3 }
    );
}

#[test]
fn classify_event_flag_create() {
    let args = [0x7000, 0x8000, 0xCAFE_BABE_DEAD_BEEF, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::EVENT_FLAG_CREATE, &args),
        Lv2Request::EventFlagCreate {
            id_ptr: 0x7000,
            attr_ptr: 0x8000,
            init: 0xCAFE_BABE_DEAD_BEEF,
        }
    );
}

#[test]
fn classify_event_flag_destroy() {
    let args = [11, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::EVENT_FLAG_DESTROY, &args),
        Lv2Request::EventFlagDestroy { id: 11 }
    );
}

#[test]
fn classify_event_flag_wait_and_trywait() {
    let wait_args = [11, 0b1010, 0x02, 0xC000, 0xFFFF_FFFF_FFFF_FFFF, 0, 0, 0];
    assert_eq!(
        classify(syscall::EVENT_FLAG_WAIT, &wait_args),
        Lv2Request::EventFlagWait {
            id: 11,
            bits: 0b1010,
            mode: 0x02,
            result_ptr: 0xC000,
            timeout: 0xFFFF_FFFF_FFFF_FFFF,
        }
    );
    let trywait_args = [11, 0b0101, 0x02, 0xC000, 0, 0, 0, 0];
    assert_eq!(
        classify(syscall::EVENT_FLAG_TRY_WAIT, &trywait_args),
        Lv2Request::EventFlagTryWait {
            id: 11,
            bits: 0b0101,
            mode: 0x02,
            result_ptr: 0xC000,
        }
    );
}

#[test]
fn classify_event_flag_set_clear_cancel_get() {
    assert_eq!(
        classify(syscall::EVENT_FLAG_SET, &[11, 0b1100, 0, 0, 0, 0, 0, 0]),
        Lv2Request::EventFlagSet {
            id: 11,
            bits: 0b1100,
        }
    );
    assert_eq!(
        classify(syscall::EVENT_FLAG_CLEAR, &[11, 0b0011, 0, 0, 0, 0, 0, 0]),
        Lv2Request::EventFlagClear {
            id: 11,
            bits: 0b0011,
        }
    );
    assert_eq!(
        classify(syscall::EVENT_FLAG_CANCEL, &[11, 0xD000, 0, 0, 0, 0, 0, 0]),
        Lv2Request::EventFlagCancel {
            id: 11,
            num_ptr: 0xD000,
        }
    );
    assert_eq!(
        classify(syscall::EVENT_FLAG_GET, &[11, 0xE000, 0, 0, 0, 0, 0, 0]),
        Lv2Request::EventFlagGet {
            id: 11,
            flags_ptr: 0xE000,
        }
    );
}
