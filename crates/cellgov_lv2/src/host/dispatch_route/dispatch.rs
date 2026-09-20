//! Top-level dispatch routing for [`Lv2Host`].
//!
//! Per-arm shape and citations live with the extracted `dispatch_*`
//! methods; shared helpers live in [`super::helpers`].

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::{
    census::{lookup as lookup_census, PupCensusClass},
    errno, syscall,
};

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
    /// host's complete response; guest-memory writes ride as
    /// `Effect`s the runtime feeds into the commit pipeline.
    /// `rt.current_tick()` is snapshotted on entry so every effect
    /// is stamped at the triggering syscall's tick.
    pub fn dispatch(
        &mut self,
        request: Lv2Request,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        self.dispatch_with_census(request, requester, rt, None)
    }

    /// Dispatch a classified syscall with its source ordinal for firmware census checks.
    ///
    /// The runtime supplies `ordinal` before request classification loses
    /// it. Direct host users that do not have an ordinal should call
    /// [`Self::dispatch`].
    pub fn dispatch_with_ordinal(
        &mut self,
        request: Lv2Request,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        ordinal: u64,
    ) -> Lv2Dispatch {
        self.dispatch_with_census(request, requester, rt, usize::try_from(ordinal).ok())
    }

    fn dispatch_with_census(
        &mut self,
        request: Lv2Request,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        ordinal: Option<usize>,
    ) -> Lv2Dispatch {
        let census_refuses = if !matches!(request, Lv2Request::Hypercall { .. }) {
            if let (Some(ordinal), Some(identity)) = (ordinal, self.firmware_identity()) {
                matches!(
                    lookup_census(&identity.pup_sha256_bytes, ordinal),
                    PupCensusClass::Absent | PupCensusClass::OutOfRange
                )
            } else {
                false
            }
        } else {
            false
        };
        let arm: &'static str = crate::request::Lv2RequestKind::from(&request).into();
        let wait_timeout = request.wait_timeout_usec();
        // Census refusals still pass the common observation point below:
        // Lv2Observability promises to count every non-zero immediate return.
        let out = if census_refuses {
            Lv2Dispatch::immediate(errno::CELL_ENOSYS.into())
        } else {
            self.dispatch_routed(request, requester, rt)
        };
        match &out {
            Lv2Dispatch::Immediate { code, .. } | Lv2Dispatch::ImmediateRegisters { code, .. }
                if *code != 0 =>
            {
                *self.obs.dispatch_nonzero_returns.entry(*code).or_insert(0) += 1;
                *self
                    .obs
                    .dispatch_return_pairs
                    .entry((arm, *code))
                    .or_insert(0) += 1;
            }
            // Both park shapes register a wake deadline (the runtime's
            // `register_wait_deadline` covers `Block` and `BlockAndWake`);
            // `sys_cond_wait` parks via `BlockAndWake` when releasing the
            // mutex transfers ownership.
            Lv2Dispatch::Block { .. } | Lv2Dispatch::BlockAndWake { .. } => {
                if let Some(timeout) = wait_timeout {
                    *self.obs.park_timeouts.entry((arm, timeout)).or_insert(0) += 1;
                }
            }
            _ => {}
        }
        out
    }

    /// Split from [`Self::dispatch`] so every arm's return passes one
    /// observation point.
    fn dispatch_routed(
        &mut self,
        request: Lv2Request,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let tick = rt.current_tick();
        match request {
            Lv2Request::SpuImageOpen { img_ptr, path_ptr } => {
                self.dispatch_image_open(img_ptr, path_ptr, requester, rt, tick)
            }
            Lv2Request::SpuImageImport {
                handle_out,
                img_ptr,
                size,
                type_id,
            } => {
                self.dispatch_image_import(handle_out, img_ptr, size, type_id, requester, rt, tick)
            }
            Lv2Request::SpuThreadGroupCreate {
                id_ptr,
                num_threads,
                ..
            } => self.dispatch_group_create(id_ptr, num_threads, requester, tick),
            req @ Lv2Request::SpuThreadInitialize { .. } => {
                self.dispatch_thread_initialize(req, requester, rt, tick)
            }
            Lv2Request::SpuThreadGroupStart { group_id } => self.dispatch_group_start(group_id),
            Lv2Request::SpuThreadGroupDestroy { id } => self.dispatch_group_destroy(id),
            Lv2Request::SpuThreadGroupJoin {
                group_id,
                cause_ptr,
                status_ptr,
            } => self.dispatch_group_join(group_id, cause_ptr, status_ptr, requester, tick),
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
            } => self.dispatch_tty_write(buf_ptr, len, nwritten_ptr, requester, rt, tick),
            Lv2Request::LwMutexCreate { id_ptr, .. } => {
                self.dispatch_lwmutex_create(id_ptr, requester, tick)
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
            } => self.dispatch_fs_open(path_ptr, flags, fd_out_ptr, mode, requester, rt, tick),
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
            } => self.dispatch_fs_opendir(path_ptr, fd_out_ptr, requester, rt, tick),
            Lv2Request::FsReaddir {
                fd,
                dirent_out_ptr,
                nread_out_ptr,
            } => self.dispatch_fs_readdir(fd, dirent_out_ptr, nread_out_ptr, requester, rt),
            Lv2Request::FsClosedir { fd } => self.dispatch_fs_closedir(fd),
            Lv2Request::FsWrite {
                fd,
                buf_ptr,
                size,
                nwrite_ptr,
            } => self.dispatch_fs_write(fd, buf_ptr, size, nwrite_ptr, requester, tick),
            Lv2Request::MutexCreate { id_ptr, attr_ptr } => {
                self.dispatch_mutex_create(id_ptr, attr_ptr, requester, rt, tick)
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
            } => {
                self.dispatch_semaphore_create(id_ptr, attr_ptr, initial, max, requester, rt, tick)
            }
            Lv2Request::SemaphoreDestroy { id } => self.dispatch_semaphore_destroy(id),
            Lv2Request::SemaphoreWait { id, .. } => self.dispatch_semaphore_wait(id, requester),
            Lv2Request::SemaphorePost { id, val } => self.dispatch_semaphore_post(id, val),
            Lv2Request::SemaphoreTryWait { id } => self.dispatch_semaphore_trywait(id),
            Lv2Request::SemaphoreGetValue { id, out_ptr } => {
                self.dispatch_semaphore_get_value(id, out_ptr, requester, tick)
            }
            Lv2Request::EventQueueCreate {
                id_ptr, key, size, ..
            } => self.dispatch_event_queue_create(id_ptr, key, size, requester, tick),
            Lv2Request::EventQueueDestroy { queue_id } => {
                self.dispatch_event_queue_destroy(queue_id)
            }
            Lv2Request::EventQueueReceive {
                queue_id, out_ptr, ..
            } => self.dispatch_event_queue_receive(queue_id, out_ptr, requester, tick),
            Lv2Request::UartInitialize => self.dispatch_uart_initialize(),
            Lv2Request::UartReceive {
                buf_ptr,
                size,
                mode,
            } => self.dispatch_uart_receive(buf_ptr, size, mode, requester, rt, tick),
            Lv2Request::UartSend {
                buf_ptr,
                size,
                mode,
            } => self.dispatch_uart_send(buf_ptr, size, mode, requester, rt, tick),
            Lv2Request::UartGetParams { params_ptr } => {
                self.dispatch_uart_get_params(params_ptr, requester, rt, tick)
            }
            Lv2Request::UsbdInitialize { handle_ptr } => {
                self.dispatch_usbd_initialize(handle_ptr, requester, tick)
            }
            Lv2Request::UsbdFinalize { handle } => {
                self.dispatch_usbd_finalize(handle, requester, tick)
            }
            Lv2Request::UsbdGetDeviceList { handle, .. } => {
                self.dispatch_usbd_get_device_list(handle)
            }
            Lv2Request::UsbdGetDescriptor {
                handle, desc_ptr, ..
            } => self.dispatch_usbd_get_descriptor(handle, desc_ptr),
            Lv2Request::UsbdGetDescriptorSize { handle, .. }
            | Lv2Request::UsbdOpenPipe { handle, .. }
            | Lv2Request::UsbdOpenDefaultPipe { handle, .. }
            | Lv2Request::UsbdClosePipe { handle, .. } => self.dispatch_usbd_no_device(handle),
            Lv2Request::UsbdRegisterLdd {
                handle,
                product_ptr,
                product_len,
            } => self.dispatch_usbd_register_ldd(handle, product_ptr, product_len, rt),
            Lv2Request::UsbdUnregisterLdd {
                handle,
                product_ptr,
                product_len,
            } => self.dispatch_usbd_unregister_ldd(handle, product_ptr, product_len, rt),
            Lv2Request::UsbdReceiveEvent {
                handle,
                arg1_ptr,
                arg2_ptr,
                arg3_ptr,
            } => self.dispatch_usbd_receive_event(
                handle,
                [arg1_ptr, arg2_ptr, arg3_ptr],
                requester,
                rt,
            ),
            Lv2Request::UsbdDetectEvent => self.dispatch_usbd_detect_event(),
            Lv2Request::ConfigOpen {
                equeue_id,
                out_handle_ptr,
            } => self.dispatch_config_open(equeue_id, out_handle_ptr, requester, tick),
            Lv2Request::ConfigClose { handle } => self.dispatch_config_close(handle),
            Lv2Request::ConfigGetServiceEvent {
                handle,
                event_id,
                dst_ptr,
                size,
            } => self.dispatch_config_get_service_event(
                handle, event_id, dst_ptr, size, requester, tick,
            ),
            Lv2Request::ConfigAddServiceListener {
                handle,
                service_id,
                min_verbosity,
                in_ptr,
                size,
                listener_type,
                out_listener_ptr,
            } => self.dispatch_config_add_service_listener(
                handle,
                crate::host::config::ListenerSpec {
                    service_id,
                    min_verbosity,
                    in_ptr,
                    size,
                    listener_type,
                },
                out_listener_ptr,
                requester,
                rt,
                tick,
            ),
            Lv2Request::ConfigRemoveServiceListener { handle, listener } => {
                self.dispatch_config_remove_service_listener(handle, listener)
            }
            Lv2Request::ConfigRegisterService {
                handle,
                service_id,
                user_id,
                verbosity,
                data_ptr,
                size,
                out_service_ptr,
            } => self.dispatch_config_register_service(
                handle,
                crate::host::config::ServiceSpec {
                    service_id,
                    user_id,
                    verbosity,
                    data_ptr,
                    size,
                },
                out_service_ptr,
                requester,
                rt,
                tick,
            ),
            Lv2Request::ConfigUnregisterService { handle, service } => {
                self.dispatch_config_unregister_service(handle, service)
            }
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
                tick,
            ),
            Lv2Request::EventFlagCreate {
                id_ptr,
                attr_ptr,
                init,
            } => self.dispatch_event_flag_create(id_ptr, attr_ptr, init, requester, rt, tick),
            Lv2Request::EventFlagDestroy { id } => self.dispatch_event_flag_destroy(id),
            Lv2Request::EventFlagWait {
                id,
                bits,
                mode,
                result_ptr,
                ..
            } => self.dispatch_event_flag_wait(id, bits, mode, result_ptr, requester, tick),
            Lv2Request::EventFlagTryWait {
                id,
                bits,
                mode,
                result_ptr,
            } => self.dispatch_event_flag_trywait(id, bits, mode, result_ptr, requester, tick),
            Lv2Request::EventFlagSet { id, bits } => self.dispatch_event_flag_set(id, bits),
            Lv2Request::EventFlagClear { id, bits } => self.dispatch_event_flag_clear(id, bits),
            Lv2Request::EventFlagCancel { id, num_ptr } => {
                self.dispatch_event_flag_cancel(id, num_ptr, requester, tick)
            }
            Lv2Request::EventFlagGet { id, flags_ptr } => {
                self.dispatch_event_flag_get(id, flags_ptr, requester, tick)
            }
            Lv2Request::CondCreate {
                id_ptr,
                mutex_id,
                attr_ptr,
            } => self.dispatch_cond_create(id_ptr, mutex_id, attr_ptr, requester, rt, tick),
            Lv2Request::CondDestroy { id } => self.dispatch_cond_destroy(id),
            Lv2Request::CondWait { id, .. } => self.dispatch_cond_wait(id, requester, rt),
            Lv2Request::CondSignal { id } => self.dispatch_cond_signal(id),
            Lv2Request::CondSignalAll { id } => self.dispatch_cond_signal_all(id),
            Lv2Request::CondSignalTo { id, target_thread } => {
                self.dispatch_cond_signal_to(id, target_thread)
            }
            Lv2Request::MemoryAllocate {
                size,
                alloc_addr_ptr,
                ..
            } => self.dispatch_memory_allocate(size, alloc_addr_ptr, requester, tick),
            Lv2Request::MemoryFree { .. } => self.dispatch_memory_free_noop(),
            Lv2Request::MemoryContainerCreate { cid_ptr, size } => {
                self.dispatch_memory_container_create(cid_ptr, size, requester, tick)
            }
            Lv2Request::MemoryAllocateFromContainer {
                size,
                cid,
                flags,
                alloc_addr_ptr,
            } => self.dispatch_memory_allocate_from_container(
                size,
                cid,
                flags,
                alloc_addr_ptr,
                requester,
                tick,
            ),
            Lv2Request::PpuThreadYield => self.dispatch_ppu_thread_yield(),
            Lv2Request::PpuThreadStart { target } => self.dispatch_ppu_thread_start(target),
            Lv2Request::TimeGetTimebaseFrequency => self.dispatch_time_get_timebase_frequency(),
            Lv2Request::TimeGetTimezone {
                timezone_ptr,
                summer_time_ptr,
            } => self.dispatch_time_get_timezone(timezone_ptr, summer_time_ptr, requester, tick),
            Lv2Request::MemoryGetUserMemorySize { mem_info_ptr } => {
                self.dispatch_memory_get_user_memory_size(mem_info_ptr, requester, tick)
            }
            Lv2Request::TimeGetCurrentTime { sec_ptr, nsec_ptr } => {
                self.dispatch_time_get_current_time(sec_ptr, nsec_ptr, requester, tick)
            }
            Lv2Request::PpuThreadExit { exit_value } => {
                self.dispatch_ppu_thread_exit(exit_value, requester)
            }
            Lv2Request::PpuThreadCreate {
                id_ptr,
                param_ptr,
                arg,
                unk,
                priority,
                stacksize,
                flags,
                threadname_ptr,
            } => self.dispatch_ppu_thread_create_with_flag_log(
                id_ptr,
                param_ptr,
                arg,
                unk,
                priority,
                stacksize,
                flags,
                threadname_ptr,
                rt,
            ),
            Lv2Request::PpuThreadJoin {
                target,
                status_out_ptr,
            } => self.dispatch_ppu_thread_join(target, status_out_ptr, requester, tick),
            Lv2Request::SysRsxMemoryAllocate {
                mem_handle_ptr,
                mem_addr_ptr,
                size,
                ..
            } => self.dispatch_sys_rsx_memory_allocate(
                mem_handle_ptr,
                mem_addr_ptr,
                size,
                requester,
                tick,
            ),
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
                tick,
            ),
            Lv2Request::SysRsxContextFree { .. } => self.dispatch_sys_rsx_context_free_noop(),
            Lv2Request::SysRsxContextAttribute {
                context_id,
                package_id,
                a3,
                a4,
                a5,
                a6,
            } => self.dispatch_sys_rsx_context_attribute(
                context_id, package_id, a3, a4, a5, a6, requester, tick, rt,
            ),
            Lv2Request::SysRsxContextIomap {
                context_id,
                io,
                ea,
                size,
                flags,
            } => self.dispatch_sys_rsx_context_iomap(context_id, io, ea, size, flags),
            Lv2Request::SysRsxDeviceMap {
                dev_addr_ptr,
                a2_ptr,
                dev_id,
            } => self.dispatch_sys_rsx_device_map(dev_addr_ptr, a2_ptr, dev_id, requester, tick),
            Lv2Request::SsAccessControlEngine { pkg_id, a2, .. } => {
                self.dispatch_ss_access_control_engine(pkg_id, a2, requester, tick)
            }
            // Both signatures place `path` at arg 0. liblv2.sprx's
            // wrappers for 480 and 497 each forward their own first
            // argument, the module path, into arg 0 unchanged. 497
            // carries an extra memory container, but it does not
            // shift that slot.
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_LOAD_MODULE,
                args,
            } => self.resolve_prx_load(syscall::SYS_PRX_LOAD_MODULE, args[0], rt),
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_LOAD_MODULE_ON_MEMCONTAINER,
                args,
            } => self.resolve_prx_load(syscall::SYS_PRX_LOAD_MODULE_ON_MEMCONTAINER, args[0], rt),
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_START_MODULE,
                args,
            } => self.dispatch_prx_start_module(args, requester, rt, tick),
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_STOP_MODULE,
                args,
            } => self.dispatch_prx_stop_module(args, requester, rt, tick),
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_UNLOAD_MODULE,
                args,
            } => self.dispatch_prx_unload_module(args),
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
                args,
            } => self.dispatch_prx_register_module(args, requester, rt, tick),
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_REGISTER_LIBRARY,
                args,
            } => self.dispatch_prx_register_library(args, rt),
            Lv2Request::Unsupported {
                number: syscall::PPU_THREAD_SET_PRIORITY,
                args,
            } => self.dispatch_ppu_thread_set_priority(args),
            Lv2Request::Unsupported {
                number: syscall::PPU_THREAD_GET_PRIORITY,
                args,
            } => self.dispatch_ppu_thread_get_priority(args, requester, tick),
            Lv2Request::Unsupported {
                number: syscall::SYS_PRX_GET_MODULE_LIST,
                args,
            } => self.dispatch_prx_get_module_list(args, requester, rt, tick),
            Lv2Request::Unsupported {
                number: syscall::EVENT_PORT_CONNECT_LOCAL,
                args,
            } => match self.narrow_u32_args(
                syscall::EVENT_PORT_CONNECT_LOCAL,
                [("event_port_id", args[0]), ("event_queue_id", args[1])],
            ) {
                Some([port_id, queue_id]) => {
                    self.dispatch_event_port_connect_local(port_id, queue_id)
                }
                None => Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
            },
            Lv2Request::Unsupported {
                number: syscall::EVENT_PORT_CONNECT_IPC,
                args,
            } => match self.narrow_u32_args(
                syscall::EVENT_PORT_CONNECT_IPC,
                [("event_port_id", args[0])],
            ) {
                Some([port_id]) => self.dispatch_event_port_connect_ipc(port_id, args[1]),
                None => Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
            },
            Lv2Request::Unsupported {
                number: syscall::EVENT_PORT_DISCONNECT,
                args,
            } => match self
                .narrow_u32_args(syscall::EVENT_PORT_DISCONNECT, [("event_port_id", args[0])])
            {
                Some([port_id]) => self.dispatch_event_port_disconnect(port_id),
                None => Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
            },
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
            } => match self
                .narrow_u32_args(syscall::MEMORY_CONTAINER_CREATE_324, [("cid", args[0])])
            {
                Some([cid_ptr]) => {
                    self.dispatch_memory_container_create(cid_ptr, args[1], requester, tick)
                }
                None => Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
            },
            Lv2Request::Unsupported {
                number: syscall::MMAPPER_ALLOCATE_ADDRESS,
                args,
            } => self.dispatch_mmapper_allocate_address(args, requester, tick),
            Lv2Request::Unsupported {
                number: syscall::MMAPPER_MAP_SHARED_MEMORY,
                args,
            } => self.dispatch_mmapper_map_shared_memory(args, requester, rt, tick),
            Lv2Request::Unsupported {
                number: syscall::MMAPPER_SEARCH_AND_MAP,
                args,
            } => self.dispatch_mmapper_search_and_map(args, requester, rt, tick),
            Lv2Request::Unsupported {
                number: syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_FROM_CONTAINER,
                args,
            } => self.dispatch_mmapper_allocate_shared_memory_from_container(args, requester, tick),
            Lv2Request::Unsupported {
                number: syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_EXT,
                args,
            } => self.dispatch_mmapper_allocate_shared_memory_ext(args, requester, rt, tick),
            Lv2Request::Unsupported {
                number: syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
                args,
            } => self.dispatch_mmapper_allocate_shared_memory(args, requester, tick),
            Lv2Request::ProcessExit { code } => self.dispatch_process_exit(code, requester),
            Lv2Request::ProcessExit2 {
                code,
                arg_ptr,
                arg_size,
                arg4,
            } => {
                if arg4 != 0 {
                    self.log_invariant_break(
                        "process.exit2_unconsumed_arg4",
                        format_args!(
                            "process exit2 carried nonzero fourth arg \
                             {arg4:#x}; not consumed",
                        ),
                    );
                }
                self.dispatch_process_exit2(code, arg_ptr, arg_size, requester, rt)
            }
            Lv2Request::ProcessSpawn {
                pid_out_ptr,
                prio,
                flags,
                block_ptr,
                block_size,
                data_word,
                unconsumed,
            } => {
                if unconsumed != [0; 2] {
                    self.log_invariant_break(
                        "process.spawn_unconsumed_args",
                        format_args!(
                            "process spawn carried nonzero trailing args \
                             [{:#x}, {:#x}] (sc 21: r8/r9 pair values; \
                             sc 27: r9 config block / r10 pair block); \
                             not consumed",
                            unconsumed[0], unconsumed[1],
                        ),
                    );
                }
                self.dispatch_process_spawn(
                    pid_out_ptr,
                    prio,
                    flags,
                    block_ptr,
                    block_size,
                    data_word,
                    requester,
                    rt,
                )
            }
            Lv2Request::ProcessGetStatus { pid } => self.dispatch_process_get_status(pid),
            Lv2Request::ProcessGetPid => self.dispatch_process_get_pid(requester),
            Lv2Request::ProcessGetPpid => self.dispatch_process_get_ppid(requester),
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
            } => {
                self.dispatch_process_get_number_of_object(class_id, count_out_ptr, requester, tick)
            }
            Lv2Request::ProcessGetSdkVersion {
                version_out_ptr, ..
            } => self.dispatch_process_get_sdk_version(version_out_ptr, requester, tick),
            Lv2Request::ProcessGetParamsfo { buf_ptr } => {
                self.dispatch_process_get_paramsfo(buf_ptr, requester, tick)
            }
            Lv2Request::TimerCreate { id_ptr } => {
                self.dispatch_timer_create(id_ptr, requester, tick)
            }
            Lv2Request::TimerDestroy { .. } => self.dispatch_timer_destroy(),
            Lv2Request::RwlockCreate { id_ptr, .. } => {
                self.dispatch_rwlock_create(id_ptr, requester, tick)
            }
            Lv2Request::RwlockDestroy { .. } => self.dispatch_rwlock_destroy(),
            Lv2Request::EventPortCreate {
                id_ptr,
                port_type,
                name,
            } => {
                self.dispatch_event_port_create(id_ptr, u64::from(port_type), name, requester, tick)
            }
            Lv2Request::EventPortDestroy { id } => self.dispatch_event_port_destroy(id),
            Lv2Request::Hypercall { lev, r11, args } => {
                self.dispatch_hypercall_rejection(lev.get(), r11, args)
            }
            Lv2Request::NoSuchSyscall { number, args } => {
                self.dispatch_no_such_syscall(number, args)
            }
            Lv2Request::Unsupported { number, args } => {
                self.dispatch_unsupported_default(number, args, tick)
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
