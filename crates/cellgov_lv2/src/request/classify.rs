use std::num::NonZeroU8;

use cellgov_ps3_abi::syscall;
use cellgov_ps3_abi::syscall_namespace::SyscallNamespace;

use super::Lv2Request;

const HIGH_BITS_REASONS: [&str; 8] = [
    "arg 0: non-zero high 32 bits in u32 field",
    "arg 1: non-zero high 32 bits in u32 field",
    "arg 2: non-zero high 32 bits in u32 field",
    "arg 3: non-zero high 32 bits in u32 field",
    "arg 4: non-zero high 32 bits in u32 field",
    "arg 5: non-zero high 32 bits in u32 field",
    "arg 6: non-zero high 32 bits in u32 field",
    "arg 7: non-zero high 32 bits in u32 field",
];

const I32_RANGE_REASONS: [&str; 8] = [
    "arg 0: not representable as i32",
    "arg 1: not representable as i32",
    "arg 2: not representable as i32",
    "arg 3: not representable as i32",
    "arg 4: not representable as i32",
    "arg 5: not representable as i32",
    "arg 6: not representable as i32",
    "arg 7: not representable as i32",
];

/// LEV=0 wrapper around [`classify_with_lev`] for synthetic / fake-ISA
/// test paths. Real PPU `sc` decode goes through `classify_with_lev`.
#[inline]
pub fn classify(syscall_num: u64, args: &[u64; 8]) -> Lv2Request {
    classify_with_lev(0, syscall_num, args)
}

/// Build an [`Lv2Request`] from the `sc` LEV field, r11, and r3..=r10.
///
/// Non-zero `lev` routes to [`Lv2Request::Hypercall`]; LV2-syscall
/// classification only runs for `lev == 0`.
pub fn classify_with_lev(lev: u8, syscall_num: u64, args: &[u64; 8]) -> Lv2Request {
    if let Some(lev) = NonZeroU8::new(lev) {
        return Lv2Request::Hypercall {
            lev,
            r11: syscall_num,
            args: *args,
        };
    }
    macro_rules! p {
        ($idx:expr) => {
            match u32::try_from(args[$idx]) {
                Ok(v) => v,
                Err(_) => {
                    return Lv2Request::Malformed {
                        number: syscall_num,
                        reason: HIGH_BITS_REASONS[$idx],
                        args: *args,
                    };
                }
            }
        };
    }
    // Reverses PPC64 sign extension: guest `int x = -1` arrives as
    // 0xFFFF_FFFF_FFFF_FFFF and decodes to -1i64. Values that aren't
    // a clean sign extension (e.g. 0x1_0000_0001 or 2^31) reject as
    // Malformed rather than wrapping.
    macro_rules! s {
        ($idx:expr) => {
            match i32::try_from(args[$idx] as i64) {
                Ok(v) => v,
                Err(_) => {
                    return Lv2Request::Malformed {
                        number: syscall_num,
                        reason: I32_RANGE_REASONS[$idx],
                        args: *args,
                    };
                }
            }
        };
    }

    // Exhaustive match so a new SyscallNamespace variant forces a
    // compile error. The UnresolvedImport namespace currently
    // carries one entry; the NID rides in r4 (args[1]).
    match SyscallNamespace::of(syscall_num) {
        Some(SyscallNamespace::UnresolvedImport) if syscall_num == syscall::UNRESOLVED_IMPORT => {
            return Lv2Request::UnresolvedImport { nid: p!(1) };
        }
        Some(SyscallNamespace::UnresolvedImport) => {
            return Lv2Request::Unsupported {
                number: syscall_num,
                args: *args,
            };
        }
        Some(SyscallNamespace::Lv2) | None => {}
    }
    match syscall_num {
        syscall::SPU_IMAGE_OPEN => Lv2Request::SpuImageOpen {
            img_ptr: p!(0),
            path_ptr: p!(1),
        },
        syscall::SPU_IMAGE_IMPORT => Lv2Request::SpuImageImport {
            handle_out: p!(0),
            img_ptr: p!(1),
            size: args[2],
            type_id: p!(3),
        },
        syscall::SPU_THREAD_GROUP_CREATE => Lv2Request::SpuThreadGroupCreate {
            id_ptr: p!(0),
            num_threads: p!(1),
            priority: s!(2),
            attr_ptr: p!(3),
        },
        syscall::SPU_THREAD_INITIALIZE => Lv2Request::SpuThreadInitialize {
            thread_ptr: p!(0),
            group_id: p!(1),
            thread_num: p!(2),
            img_ptr: p!(3),
            attr_ptr: p!(4),
            arg_ptr: p!(5),
        },
        syscall::SPU_THREAD_GROUP_START => Lv2Request::SpuThreadGroupStart { group_id: p!(0) },
        syscall::SPU_THREAD_GROUP_DESTROY => Lv2Request::SpuThreadGroupDestroy { id: p!(0) },
        syscall::SPU_THREAD_GROUP_JOIN => Lv2Request::SpuThreadGroupJoin {
            group_id: p!(0),
            cause_ptr: p!(1),
            status_ptr: p!(2),
        },
        syscall::SPU_THREAD_GROUP_TERMINATE => Lv2Request::SpuThreadGroupTerminate {
            group_id: p!(0),
            value: s!(1),
        },
        syscall::SPU_THREAD_WRITE_MB => Lv2Request::SpuThreadWriteMb {
            thread_id: p!(0),
            value: p!(1),
        },
        syscall::TIME_GET_TIMEZONE => Lv2Request::TimeGetTimezone {
            timezone_ptr: p!(0),
            summer_time_ptr: p!(1),
        },
        syscall::TIME_GET_CURRENT_TIME => Lv2Request::TimeGetCurrentTime {
            sec_ptr: p!(0),
            nsec_ptr: p!(1),
        },
        syscall::TIME_GET_TIMEBASE_FREQUENCY => Lv2Request::TimeGetTimebaseFrequency,
        syscall::TTY_WRITE => Lv2Request::TtyWrite {
            fd: p!(0),
            buf_ptr: p!(1),
            len: p!(2),
            nwritten_ptr: p!(3),
        },
        syscall::PROCESS_EXIT => Lv2Request::ProcessExit { code: s!(0) },
        syscall::PROCESS_GETPID => Lv2Request::ProcessGetPid,
        syscall::PROCESS_GET_NUMBER_OF_OBJECT => Lv2Request::ProcessGetNumberOfObject {
            class_id: p!(0),
            count_out_ptr: p!(1),
        },
        syscall::PROCESS_GETPPID => Lv2Request::ProcessGetPpid,
        syscall::PROCESS_GET_SDK_VERSION => Lv2Request::ProcessGetSdkVersion {
            pid: p!(0),
            version_out_ptr: p!(1),
        },
        syscall::PROCESS_GET_PARAMSFO => Lv2Request::ProcessGetParamsfo { buf_ptr: p!(0) },
        syscall::PROCESS_GET_PPU_GUID => Lv2Request::ProcessGetPpuGuid,
        syscall::PROCESS_IS_SPU_LOCK_LINE_RESERVATION_ADDRESS => {
            Lv2Request::ProcessIsSpuLockLineReservationAddress {
                addr: p!(0),
                flags: args[1],
            }
        }
        syscall::SPU_INITIALIZE => Lv2Request::SpuInitialize {
            max_usable_spu: p!(0),
            max_raw_spu: p!(1),
        },
        syscall::TIMER_CREATE => Lv2Request::TimerCreate { id_ptr: p!(0) },
        syscall::TIMER_DESTROY => Lv2Request::TimerDestroy { id: p!(0) },
        syscall::RWLOCK_CREATE => Lv2Request::RwlockCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
        },
        syscall::RWLOCK_DESTROY => Lv2Request::RwlockDestroy { id: p!(0) },
        syscall::EVENT_PORT_CREATE => Lv2Request::EventPortCreate {
            id_ptr: p!(0),
            port_type: p!(1),
            name: args[2],
        },
        syscall::EVENT_PORT_DESTROY => Lv2Request::EventPortDestroy { id: p!(0) },
        syscall::PPU_THREAD_YIELD => Lv2Request::PpuThreadYield,
        syscall::PPU_THREAD_START => Lv2Request::PpuThreadStart { target: args[0] },
        syscall::PPU_THREAD_EXIT => Lv2Request::PpuThreadExit {
            exit_value: args[0],
        },
        syscall::PPU_THREAD_CREATE => Lv2Request::PpuThreadCreate {
            id_ptr: p!(0),
            param_ptr: p!(1),
            arg: args[2],
            priority: s!(3),
            stacksize: args[4],
            flags: args[5],
        },
        syscall::SYS_RSX_MEMORY_ALLOCATE => Lv2Request::SysRsxMemoryAllocate {
            mem_handle_ptr: p!(0),
            mem_addr_ptr: p!(1),
            size: p!(2),
            flags: args[3],
            a5: args[4],
            a6: args[5],
            a7: args[6],
        },
        syscall::SYS_RSX_MEMORY_FREE => Lv2Request::SysRsxMemoryFree { mem_handle: p!(0) },
        syscall::SYS_RSX_CONTEXT_ALLOCATE => Lv2Request::SysRsxContextAllocate {
            context_id_ptr: p!(0),
            lpar_dma_control_ptr: p!(1),
            lpar_driver_info_ptr: p!(2),
            lpar_reports_ptr: p!(3),
            mem_ctx: args[4],
            system_mode: args[5],
        },
        syscall::SYS_RSX_CONTEXT_FREE => Lv2Request::SysRsxContextFree { context_id: p!(0) },
        syscall::SYS_RSX_CONTEXT_ATTRIBUTE => Lv2Request::SysRsxContextAttribute {
            context_id: p!(0),
            package_id: p!(1),
            a3: args[2],
            a4: args[3],
            a5: args[4],
            a6: args[5],
        },
        syscall::SYS_RSX_CONTEXT_IOMAP => Lv2Request::SysRsxContextIomap {
            context_id: p!(0),
            io: p!(1),
            ea: p!(2),
            size: p!(3),
            flags: args[4],
        },
        syscall::SYS_RSX_DEVICE_MAP => Lv2Request::SysRsxDeviceMap {
            dev_addr_ptr: p!(0),
            a2_ptr: p!(1),
            dev_id: p!(2),
        },
        syscall::SS_ACCESS_CONTROL_ENGINE => Lv2Request::SsAccessControlEngine {
            pkg_id: args[0],
            a2: args[1],
            a3: args[2],
        },
        syscall::PPU_THREAD_JOIN => Lv2Request::PpuThreadJoin {
            target: args[0],
            status_out_ptr: p!(1),
        },
        syscall::LWMUTEX_CREATE => Lv2Request::LwMutexCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
        },
        syscall::LWMUTEX_DESTROY => Lv2Request::LwMutexDestroy { id: p!(0) },
        syscall::LWMUTEX_LOCK => Lv2Request::LwMutexLock {
            id: p!(0),
            // Raw syscall path: no user-space struct pointer (only
            // the HLE wrapper carries one). See LwMutexLock docs.
            mutex_ptr: 0,
            timeout: args[1],
        },
        syscall::LWMUTEX_UNLOCK => Lv2Request::LwMutexUnlock { id: p!(0) },
        syscall::LWMUTEX_TRYLOCK => Lv2Request::LwMutexTryLock { id: p!(0) },
        syscall::FS_CLOSE => Lv2Request::FsClose { fd: p!(0) },
        syscall::FS_READ => Lv2Request::FsRead {
            fd: p!(0),
            buf_ptr: p!(1),
            nbytes: args[2],
            nread_out_ptr: p!(3),
        },
        syscall::FS_LSEEK => Lv2Request::FsLseek {
            fd: p!(0),
            offset: args[1] as i64,
            whence: p!(2),
            pos_out_ptr: p!(3),
        },
        syscall::FS_FSTAT => Lv2Request::FsFstat {
            fd: p!(0),
            stat_out_ptr: p!(1),
        },
        syscall::FS_STAT => Lv2Request::FsStat {
            path_ptr: p!(0),
            stat_out_ptr: p!(1),
        },
        syscall::FS_OPENDIR => Lv2Request::FsOpendir {
            path_ptr: p!(0),
            fd_out_ptr: p!(1),
        },
        syscall::FS_READDIR => Lv2Request::FsReaddir {
            fd: p!(0),
            dirent_out_ptr: p!(1),
            nread_out_ptr: p!(2),
        },
        syscall::FS_CLOSEDIR => Lv2Request::FsClosedir { fd: p!(0) },
        syscall::FS_WRITE => Lv2Request::FsWrite {
            fd: p!(0),
            buf_ptr: p!(1),
            size: args[2],
            nwrite_ptr: p!(3),
        },
        syscall::FS_OPEN => Lv2Request::FsOpen {
            path_ptr: p!(0),
            flags: p!(1),
            fd_out_ptr: p!(2),
            mode: p!(3),
        },
        syscall::MUTEX_CREATE => Lv2Request::MutexCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
        },
        syscall::MUTEX_DESTROY => Lv2Request::MutexDestroy { mutex_id: p!(0) },
        syscall::MUTEX_LOCK => Lv2Request::MutexLock {
            mutex_id: p!(0),
            timeout: args[1],
        },
        syscall::MUTEX_UNLOCK => Lv2Request::MutexUnlock { mutex_id: p!(0) },
        syscall::MUTEX_TRYLOCK => Lv2Request::MutexTryLock { mutex_id: p!(0) },
        syscall::SEMAPHORE_CREATE => Lv2Request::SemaphoreCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
            initial: s!(2),
            max: s!(3),
        },
        syscall::SEMAPHORE_DESTROY => Lv2Request::SemaphoreDestroy { id: p!(0) },
        syscall::SEMAPHORE_WAIT => Lv2Request::SemaphoreWait {
            id: p!(0),
            timeout: args[1],
        },
        syscall::SEMAPHORE_POST => Lv2Request::SemaphorePost {
            id: p!(0),
            val: s!(1),
        },
        syscall::SEMAPHORE_TRY_WAIT => Lv2Request::SemaphoreTryWait { id: p!(0) },
        syscall::SEMAPHORE_GET_VALUE => Lv2Request::SemaphoreGetValue {
            id: p!(0),
            out_ptr: p!(1),
        },
        syscall::EVENT_QUEUE_CREATE => Lv2Request::EventQueueCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
            key: args[2],
            size: p!(3),
        },
        syscall::EVENT_QUEUE_DESTROY => Lv2Request::EventQueueDestroy { queue_id: p!(0) },
        syscall::EVENT_QUEUE_RECEIVE => Lv2Request::EventQueueReceive {
            queue_id: p!(0),
            out_ptr: p!(1),
            timeout: args[2],
        },
        syscall::EVENT_FLAG_CREATE => Lv2Request::EventFlagCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
            init: args[2],
        },
        syscall::EVENT_FLAG_DESTROY => Lv2Request::EventFlagDestroy { id: p!(0) },
        syscall::EVENT_FLAG_WAIT => Lv2Request::EventFlagWait {
            id: p!(0),
            bits: args[1],
            mode: p!(2),
            result_ptr: p!(3),
            timeout: args[4],
        },
        syscall::EVENT_FLAG_TRY_WAIT => Lv2Request::EventFlagTryWait {
            id: p!(0),
            bits: args[1],
            mode: p!(2),
            result_ptr: p!(3),
        },
        syscall::EVENT_FLAG_SET => Lv2Request::EventFlagSet {
            id: p!(0),
            bits: args[1],
        },
        syscall::EVENT_FLAG_CLEAR => Lv2Request::EventFlagClear {
            id: p!(0),
            bits: args[1],
        },
        syscall::EVENT_FLAG_CANCEL => Lv2Request::EventFlagCancel {
            id: p!(0),
            num_ptr: p!(1),
        },
        syscall::EVENT_FLAG_GET => Lv2Request::EventFlagGet {
            id: p!(0),
            flags_ptr: p!(1),
        },
        syscall::EVENT_QUEUE_TRY_RECEIVE => Lv2Request::EventQueueTryReceive {
            queue_id: p!(0),
            event_array: p!(1),
            size: p!(2),
            count_out: p!(3),
        },
        syscall::EVENT_PORT_SEND => Lv2Request::EventPortSend {
            port_id: p!(0),
            data1: args[1],
            data2: args[2],
            data3: args[3],
        },
        syscall::COND_CREATE => Lv2Request::CondCreate {
            id_ptr: p!(0),
            mutex_id: p!(1),
            attr_ptr: p!(2),
        },
        syscall::COND_DESTROY => Lv2Request::CondDestroy { id: p!(0) },
        syscall::COND_WAIT => Lv2Request::CondWait {
            id: p!(0),
            timeout: args[1],
        },
        syscall::COND_SIGNAL => Lv2Request::CondSignal { id: p!(0) },
        syscall::COND_SIGNAL_ALL => Lv2Request::CondSignalAll { id: p!(0) },
        syscall::COND_SIGNAL_TO => Lv2Request::CondSignalTo {
            id: p!(0),
            target_thread: p!(1),
        },
        syscall::MEMORY_ALLOCATE => Lv2Request::MemoryAllocate {
            size: args[0],
            flags: args[1],
            alloc_addr_ptr: p!(2),
        },
        syscall::MEMORY_FREE => Lv2Request::MemoryFree { addr: p!(0) },
        syscall::MEMORY_GET_USER_MEMORY_SIZE => Lv2Request::MemoryGetUserMemorySize {
            mem_info_ptr: p!(0),
        },
        syscall::MEMORY_CONTAINER_CREATE => Lv2Request::MemoryContainerCreate {
            cid_ptr: p!(0),
            size: args[1],
        },
        n => Lv2Request::Unsupported {
            number: n,
            args: *args,
        },
    }
}

#[cfg(test)]
#[path = "tests/classify_tests.rs"]
mod tests;
