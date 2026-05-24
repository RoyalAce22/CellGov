//! Top-level dispatch routing for [`Lv2Host`].
//!
//! Every arm is a one-line delegation: typed `Lv2Request` variants
//! route into the matching `dispatch_*` method (mostly defined in
//! sibling submodules), and `Unsupported { number: N }` arms route
//! into per-syscall methods in [`super::unsupported_arms`]. The match
//! itself stays here as the routing surface; the per-arm shape and
//! oracle citations live with the extracted methods.
//!
//! Helpers that several arms share (TTY-write fast path,
//! id-out-pointer write shape, null-pointer EFAULT short-circuit,
//! PRX path resolver) live in [`super::helpers`].

use cellgov_event::UnitId;
use cellgov_ps3_abi::syscall;

use crate::dispatch::Lv2Dispatch;
use crate::request::Lv2Request;

use crate::host::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    /// Dispatch one syscall request.
    ///
    /// # Cross-module contract
    ///
    /// Called once per PPU syscall yield, synchronously inside the
    /// runtime's `step()`. The returned [`Lv2Dispatch`] is the
    /// host's complete response: any guest-memory writes ride as
    /// `Effect`s the runtime feeds into the commit pipeline. The
    /// host snapshots `rt.current_tick()` on entry so every effect
    /// it builds is stamped at the triggering syscall's tick rather
    /// than tick 0.
    pub fn dispatch(
        &mut self,
        request: Lv2Request,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        self.current_tick = rt.current_tick();
        match request {
            Lv2Request::SpuImageOpen { img_ptr, path_ptr } => {
                self.dispatch_image_open(img_ptr, path_ptr, requester, rt)
            }
            Lv2Request::SpuImageImport {
                handle_out,
                img_ptr,
                size,
                type_id,
            } => self.dispatch_image_import(handle_out, img_ptr, size, type_id, requester, rt),
            Lv2Request::SpuThreadGroupCreate {
                id_ptr,
                num_threads,
                ..
            } => self.dispatch_group_create(id_ptr, num_threads, requester),
            req @ Lv2Request::SpuThreadInitialize { .. } => {
                self.dispatch_thread_initialize(req, requester, rt)
            }
            Lv2Request::SpuThreadGroupStart { group_id } => self.dispatch_group_start(group_id),
            Lv2Request::SpuThreadGroupDestroy { id } => self.dispatch_group_destroy(id),
            Lv2Request::SpuThreadGroupJoin {
                group_id,
                cause_ptr,
                status_ptr,
            } => self.dispatch_group_join(group_id, cause_ptr, status_ptr, requester),
            Lv2Request::SpuThreadGroupTerminate { group_id, value } => {
                self.dispatch_spu_thread_group_terminate_stub(group_id, value)
            }
            Lv2Request::SpuThreadWriteMb { thread_id, value } => {
                self.dispatch_write_mb(thread_id, value, requester)
            }
            Lv2Request::TtyWrite {
                buf_ptr,
                len,
                nwritten_ptr,
                ..
            } => self.dispatch_tty_write(buf_ptr, len, nwritten_ptr, requester, rt),
            Lv2Request::LwMutexCreate { id_ptr, .. } => {
                self.dispatch_lwmutex_create(id_ptr, requester)
            }
            Lv2Request::LwMutexDestroy { id } => self.dispatch_lwmutex_destroy(id),
            Lv2Request::LwMutexLock { id, mutex_ptr, .. } => {
                self.dispatch_lwmutex_lock(id, mutex_ptr, requester)
            }
            Lv2Request::LwMutexUnlock { id } => self.dispatch_lwmutex_unlock(id, requester),
            Lv2Request::LwMutexTryLock { id } => self.dispatch_lwmutex_trylock(id, requester),
            Lv2Request::FsOpen {
                path_ptr,
                flags,
                fd_out_ptr,
                mode,
            } => self.dispatch_fs_open(path_ptr, flags, fd_out_ptr, mode, requester, rt),
            Lv2Request::FsClose { fd } => self.dispatch_fs_close(fd),
            Lv2Request::FsRead {
                fd,
                buf_ptr,
                nbytes,
                nread_out_ptr,
            } => self.dispatch_fs_read(fd, buf_ptr, nbytes, nread_out_ptr, requester, rt),
            Lv2Request::FsLseek {
                fd,
                offset,
                whence,
                pos_out_ptr,
            } => self.dispatch_fs_lseek(fd, offset, whence, pos_out_ptr, requester, rt),
            Lv2Request::FsFstat { fd, stat_out_ptr } => {
                self.dispatch_fs_fstat(fd, stat_out_ptr, requester, rt)
            }
            Lv2Request::FsStat {
                path_ptr,
                stat_out_ptr,
            } => self.dispatch_fs_stat(path_ptr, stat_out_ptr, requester, rt),
            Lv2Request::FsOpendir {
                path_ptr,
                fd_out_ptr,
            } => self.dispatch_fs_opendir(path_ptr, fd_out_ptr, requester, rt),
            Lv2Request::FsReaddir {
                fd,
                dirent_out_ptr,
                nread_out_ptr,
            } => self.dispatch_fs_readdir(fd, dirent_out_ptr, nread_out_ptr, requester, rt),
            Lv2Request::FsClosedir { fd } => self.dispatch_fs_closedir(fd),
            Lv2Request::FsWrite {
                buf_ptr,
                size,
                nwrite_ptr,
                ..
            } => {
                // Shared with TtyWrite which carries len as u32;
                // sys_fs_write's u64 size is clamped here. Real
                // tty-append writes are well under 4 GiB.
                let len = u32::try_from(size).unwrap_or(u32::MAX);
                self.dispatch_tty_write(buf_ptr, len, nwrite_ptr, requester, rt)
            }
            Lv2Request::MutexCreate { id_ptr, attr_ptr } => {
                self.dispatch_mutex_create(id_ptr, attr_ptr, requester, rt)
            }
            Lv2Request::MutexDestroy { mutex_id } => self.dispatch_mutex_destroy(mutex_id),
            Lv2Request::MutexLock { mutex_id, .. } => self.dispatch_mutex_lock(mutex_id, requester),
            Lv2Request::MutexUnlock { mutex_id } => self.dispatch_mutex_unlock(mutex_id, requester),
            Lv2Request::MutexTryLock { mutex_id } => {
                self.dispatch_mutex_trylock(mutex_id, requester)
            }
            Lv2Request::SemaphoreCreate {
                id_ptr,
                attr_ptr,
                initial,
                max,
            } => self.dispatch_semaphore_create(id_ptr, attr_ptr, initial, max, requester, rt),
            Lv2Request::SemaphoreDestroy { id } => self.dispatch_semaphore_destroy(id),
            Lv2Request::SemaphoreWait { id, timeout } => {
                self.dispatch_semaphore_wait(id, timeout, requester)
            }
            Lv2Request::SemaphorePost { id, val } => self.dispatch_semaphore_post(id, val),
            Lv2Request::SemaphoreTryWait { id } => self.dispatch_semaphore_trywait(id),
            Lv2Request::SemaphoreGetValue { id, out_ptr } => {
                self.dispatch_semaphore_get_value(id, out_ptr, requester)
            }
            Lv2Request::EventQueueCreate { id_ptr, size, .. } => {
                self.dispatch_event_queue_create(id_ptr, size, requester)
            }
            Lv2Request::EventQueueDestroy { queue_id } => {
                self.dispatch_event_queue_destroy(queue_id)
            }
            Lv2Request::EventQueueReceive {
                queue_id, out_ptr, ..
            } => self.dispatch_event_queue_receive(queue_id, out_ptr, requester),
            Lv2Request::EventPortSend {
                port_id,
                data1,
                data2,
                data3,
            } => self.dispatch_event_port_send(port_id, data1, data2, data3),
            Lv2Request::EventQueueTryReceive {
                queue_id,
                event_array,
                size,
                count_out,
            } => self.dispatch_event_queue_tryreceive(
                queue_id,
                event_array,
                size,
                count_out,
                requester,
            ),
            Lv2Request::EventFlagCreate {
                id_ptr,
                attr_ptr,
                init,
            } => self.dispatch_event_flag_create(id_ptr, attr_ptr, init, requester, rt),
            Lv2Request::EventFlagDestroy { id } => self.dispatch_event_flag_destroy(id),
            Lv2Request::EventFlagWait {
                id,
                bits,
                mode,
                result_ptr,
                timeout,
            } => self.dispatch_event_flag_wait(id, bits, mode, result_ptr, timeout, requester),
            Lv2Request::EventFlagTryWait {
                id,
                bits,
                mode,
                result_ptr,
            } => self.dispatch_event_flag_trywait(id, bits, mode, result_ptr, requester),
            Lv2Request::EventFlagSet { id, bits } => self.dispatch_event_flag_set(id, bits),
            Lv2Request::EventFlagClear { id, bits } => self.dispatch_event_flag_clear(id, bits),
            Lv2Request::EventFlagCancel { id, num_ptr } => {
                self.dispatch_event_flag_cancel(id, num_ptr, requester)
            }
            Lv2Request::EventFlagGet { id, flags_ptr } => {
                self.dispatch_event_flag_get(id, flags_ptr, requester)
            }
            Lv2Request::CondCreate {
                id_ptr, mutex_id, ..
            } => self.dispatch_cond_create(id_ptr, mutex_id, requester),
            Lv2Request::CondDestroy { id } => self.dispatch_cond_destroy(id),
            Lv2Request::CondWait { id, .. } => self.dispatch_cond_wait(id, requester),
            Lv2Request::CondSignal { id } => self.dispatch_cond_signal(id),
            Lv2Request::CondSignalAll { id } => self.dispatch_cond_signal_all(id),
            Lv2Request::CondSignalTo { id, target_thread } => {
                self.dispatch_cond_signal_to(id, target_thread)
            }
            Lv2Request::MemoryAllocate {
                size,
                alloc_addr_ptr,
                ..
            } => self.dispatch_memory_allocate(size, alloc_addr_ptr, requester),
            Lv2Request::MemoryFree { .. } => self.dispatch_memory_free_noop(),
            Lv2Request::MemoryContainerCreate { cid_ptr, .. } => {
                self.dispatch_memory_container_create(cid_ptr, requester)
            }
            Lv2Request::PpuThreadYield => self.dispatch_ppu_thread_yield(),
            Lv2Request::PpuThreadStart { target } => self.dispatch_ppu_thread_start(target),
            Lv2Request::TimeGetTimebaseFrequency => self.dispatch_time_get_timebase_frequency(),
            Lv2Request::TimeGetTimezone {
                timezone_ptr,
                summer_time_ptr,
            } => self.dispatch_time_get_timezone(timezone_ptr, summer_time_ptr, requester),
            Lv2Request::MemoryGetUserMemorySize { mem_info_ptr } => {
                self.dispatch_memory_get_user_memory_size(mem_info_ptr, requester)
            }
            Lv2Request::TimeGetCurrentTime { sec_ptr, nsec_ptr } => {
                self.dispatch_time_get_current_time(sec_ptr, nsec_ptr, requester)
            }
            Lv2Request::PpuThreadExit { exit_value } => {
                self.dispatch_ppu_thread_exit(exit_value, requester)
            }
            Lv2Request::PpuThreadCreate {
                id_ptr,
                param_ptr,
                arg,
                priority,
                stacksize,
                flags,
            } => self.dispatch_ppu_thread_create_with_flag_log(
                id_ptr, param_ptr, arg, priority, stacksize, flags, rt,
            ),
            Lv2Request::PpuThreadJoin {
                target,
                status_out_ptr,
            } => self.dispatch_ppu_thread_join(target, status_out_ptr, requester),
            Lv2Request::SysRsxMemoryAllocate {
                mem_handle_ptr,
                mem_addr_ptr,
                size,
                ..
            } => {
                self.dispatch_sys_rsx_memory_allocate(mem_handle_ptr, mem_addr_ptr, size, requester)
            }
            Lv2Request::SysRsxMemoryFree { .. } => self.dispatch_sys_rsx_memory_free_noop(),
            Lv2Request::SysRsxContextAllocate {
                context_id_ptr,
                lpar_dma_control_ptr,
                lpar_driver_info_ptr,
                lpar_reports_ptr,
                mem_ctx,
                system_mode,
            } => self.dispatch_sys_rsx_context_allocate(
                context_id_ptr,
                lpar_dma_control_ptr,
                lpar_driver_info_ptr,
                lpar_reports_ptr,
                mem_ctx,
                system_mode,
                requester,
            ),
            Lv2Request::SysRsxContextFree { .. } => self.dispatch_sys_rsx_context_free_noop(),
            Lv2Request::SysRsxContextAttribute {
                context_id,
                package_id,
                a3,
                a4,
                a5,
                a6,
            } => self.dispatch_sys_rsx_context_attribute(context_id, package_id, a3, a4, a5, a6),
            Lv2Request::SsAccessControlEngine { pkg_id, a2, .. } => {
                self.dispatch_ss_access_control_engine(pkg_id, a2, requester)
            }
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_LOAD_MODULE,
                args,
            } => self.resolve_prx_load(args[0], rt),
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_LOAD_MODULE_ON_MEMCONTAINER,
                args,
            } => self.resolve_prx_load(args[0], rt),
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_START_MODULE,
                args,
            } => self.dispatch_prx_start_module(args, requester),
            Lv2Request::Unsupported {
                number: syscall::TTY_READ,
                ..
            } => self.dispatch_tty_read(),
            Lv2Request::Unsupported {
                number: syscall::UNS_FUNC_462,
                ..
            } => self.dispatch_uns_func_462(),
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_REGISTER_MODULE,
                ..
            } => self.dispatch_prx_register_module(),
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_REGISTER_LIBRARY,
                ..
            } => self.dispatch_prx_register_library(),
            Lv2Request::Unsupported {
                number: syscall::PPU_THREAD_GET_PRIORITY,
                args,
            } => self.dispatch_ppu_thread_get_priority(args, requester),
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_GET_MODULE_LIST,
                args,
            } => self.dispatch_prx_get_module_list(args, requester, rt),
            Lv2Request::Unsupported {
                number: syscall::EVENT_PORT_CONNECT_LOCAL,
                ..
            } => self.dispatch_event_port_connect_local(),
            Lv2Request::Unsupported {
                number: syscall::GAMEPAD_YCON_IF,
                ..
            } => self.dispatch_gamepad_ycon_if(),
            Lv2Request::Unsupported {
                number: syscall::HID_IS_ROOT,
                ..
            } => self.dispatch_hid_is_root(),
            Lv2Request::Unsupported {
                number: syscall::RSX_ATTRIBUTE,
                ..
            } => self.dispatch_rsx_attribute(),
            Lv2Request::Unsupported {
                number: syscall::MEMORY_CONTAINER_CREATE_324,
                args,
            } => self.dispatch_memory_container_create_324(args, requester),
            Lv2Request::Unsupported {
                number: syscall::MMAPPER_ALLOCATE_ADDRESS,
                args,
            } => self.dispatch_mmapper_allocate_address(args, requester),
            Lv2Request::Unsupported {
                number: syscall::MMAPPER_MAP_SHARED_MEMORY,
                ..
            } => self.dispatch_mmapper_map_shared_memory(),
            Lv2Request::Unsupported {
                number: syscall::MMAPPER_SEARCH_AND_MAP,
                args,
            } => self.dispatch_mmapper_search_and_map(args, requester),
            Lv2Request::Unsupported {
                number: syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_FROM_CONTAINER,
                args,
            } => self.dispatch_mmapper_allocate_shared_memory_from_container(args, requester),
            Lv2Request::Unsupported {
                number: syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
                args,
            } => self.dispatch_mmapper_allocate_shared_memory(args, requester),
            Lv2Request::ProcessExit { .. } => self.dispatch_process_exit(),
            Lv2Request::ProcessGetPid => self.dispatch_process_get_pid(),
            Lv2Request::ProcessGetPpid => self.dispatch_process_get_ppid(),
            Lv2Request::ProcessGetPpuGuid => self.dispatch_process_get_ppu_guid(),
            Lv2Request::ProcessIsStack { addr } => self.dispatch_process_is_stack(addr),
            Lv2Request::ProcessIsSpuLockLineReservationAddress { addr, flags } => {
                self.dispatch_process_is_spu_lock_line_reservation_address(addr, flags)
            }
            Lv2Request::SpuInitialize {
                max_usable_spu,
                max_raw_spu,
            } => self.dispatch_spu_initialize(max_usable_spu, max_raw_spu),
            Lv2Request::ProcessGetNumberOfObject {
                class_id,
                count_out_ptr,
            } => self.dispatch_process_get_number_of_object(class_id, count_out_ptr, requester),
            Lv2Request::ProcessGetSdkVersion {
                version_out_ptr, ..
            } => self.dispatch_process_get_sdk_version(version_out_ptr, requester),
            Lv2Request::ProcessGetParamsfo { buf_ptr } => {
                self.dispatch_process_get_paramsfo(buf_ptr, requester)
            }
            // ID-allocator stubs for primitives whose only test-level
            // exercise is create/destroy plus the live-count probe.
            Lv2Request::TimerCreate { id_ptr } => self.dispatch_timer_create(id_ptr, requester),
            Lv2Request::TimerDestroy { .. } => self.dispatch_timer_destroy(),
            Lv2Request::RwlockCreate { id_ptr, .. } => {
                self.dispatch_rwlock_create(id_ptr, requester)
            }
            Lv2Request::RwlockDestroy { .. } => self.dispatch_rwlock_destroy(),
            Lv2Request::EventPortCreate { id_ptr, .. } => {
                self.dispatch_event_port_create(id_ptr, requester)
            }
            Lv2Request::EventPortDestroy { .. } => self.dispatch_event_port_destroy(),
            Lv2Request::Hypercall { lev, r11, args } => {
                self.dispatch_hypercall_rejection(lev.get(), r11, args)
            }
            Lv2Request::Unsupported { number, args } => {
                self.dispatch_unsupported_default(number, args)
            }
            Lv2Request::Malformed {
                number,
                reason,
                args,
            } => self.dispatch_malformed_rejection(number, reason, args),
            Lv2Request::UnresolvedImport { nid } => self.dispatch_unresolved_import(nid, requester),
        }
    }
}
