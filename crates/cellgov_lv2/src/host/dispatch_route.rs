//! Top-level dispatch routing for [`Lv2Host`].
//!
//! Every arm is either a one-line delegation into a submodule's
//! `dispatch_*` helper or a small inline `Lv2Dispatch::Immediate`.
//! Inline arms are kept here when their shape is "construct one or
//! two writes and an OK return"; anything more substantial belongs
//! in a primitive-specific submodule.
//!
//! [`Self::dispatch_tty_write`] and [`Self::immediate_write_u32`]
//! ride along because they are this layer's own building blocks --
//! the TTY-write fast path is reachable from both `TtyWrite` and
//! `FsWrite`, and the u32-out-pointer write shape is the building
//! block for the create-style syscalls' "alloc id + write to ptr"
//! pattern.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::request::Lv2Request;

use super::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    /// Dispatch one syscall request.
    ///
    /// # Cross-module contract
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
            Lv2Request::SpuThreadGroupJoin {
                group_id,
                cause_ptr,
                status_ptr,
            } => self.dispatch_group_join(group_id, cause_ptr, status_ptr, requester),
            Lv2Request::SpuThreadGroupTerminate { group_id, value } => {
                self.log_invariant_break(
                    "dispatch.spu_thread_group_terminate_stub",
                    format_args!(
                        "sys_spu_thread_group_terminate(group_id={group_id}, value={value}) \
                         stubbed; no SPU teardown performed"
                    ),
                );
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
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
            } => self.dispatch_tty_write(buf_ptr, size, nwrite_ptr, requester, rt),
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
            Lv2Request::MemoryFree { .. } => {
                // No dealloc tracking; titles keying on free's
                // errno will misbehave.
                Lv2Dispatch::Immediate {
                    code: 0u64,
                    effects: vec![],
                }
            }
            Lv2Request::MemoryContainerCreate { cid_ptr, .. } => {
                let id = self.alloc_id();
                self.immediate_write_u32(id, cid_ptr, requester)
            }
            // The round-robin walk advances on the syscall yield
            // itself, so the host has nothing further to do.
            Lv2Request::PpuThreadYield => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Lv2Request::TimeGetTimebaseFrequency => Lv2Dispatch::Immediate {
                code: cellgov_time::CELL_PPU_TIMEBASE_HZ,
                effects: vec![],
            },
            Lv2Request::TimeGetTimezone {
                timezone_ptr,
                summer_time_ptr,
            } => {
                let zero = 0i32.to_be_bytes();
                let tz_write = Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(timezone_ptr, 4),
                    bytes: WritePayload::from_slice(&zero),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                let dst_write = Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(summer_time_ptr, 4),
                    bytes: WritePayload::from_slice(&zero),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![tz_write, dst_write],
                }
            }
            Lv2Request::MemoryGetUserMemorySize { mem_info_ptr } => {
                let total = cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL;
                let available = total;
                let mut bytes = [0u8; 8];
                bytes[0..4].copy_from_slice(&total.to_be_bytes());
                bytes[4..8].copy_from_slice(&available.to_be_bytes());
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(mem_info_ptr, 8),
                    bytes: WritePayload::from_slice(&bytes),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            Lv2Request::TimeGetCurrentTime { sec_ptr, nsec_ptr } => {
                let (sec, nsec) = cellgov_time::ticks_to_sec_nsec(rt.current_tick().raw());
                let sec_write = Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(sec_ptr, 8),
                    bytes: WritePayload::from_slice(&sec.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                let nsec_write = Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(nsec_ptr, 8),
                    bytes: WritePayload::from_slice(&nsec.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![sec_write, nsec_write],
                }
            }
            Lv2Request::PpuThreadExit { exit_value } => {
                self.dispatch_ppu_thread_exit(exit_value, requester)
            }
            Lv2Request::PpuThreadCreate {
                id_ptr,
                entry_opd,
                arg,
                priority,
                stacksize,
                flags: _,
            } => self.dispatch_ppu_thread_create(id_ptr, entry_opd, arg, priority, stacksize, rt),
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
            // _sys_prx_start_module: liblv2 calls this with id=0
            // (our _sys_prx_load_module stub returns 0); CELL_OK
            // leaves it reading uninitialized stack. Real LV2
            // returns EINVAL for id=0/null pOpt.
            Lv2Request::Unsupported { number: 481, .. } => Lv2Dispatch::Immediate {
                code: errno::CELL_EINVAL.into(),
                effects: vec![],
            },
            // sys_tty_read: CELL_OK spins CRT input loops forever;
            // real LV2 returns EIO outside debug console mode.
            Lv2Request::Unsupported { number: 402, .. } => Lv2Dispatch::Immediate {
                code: errno::CELL_EIO.into(),
                effects: vec![],
            },
            Lv2Request::ProcessExit { .. } => self.dispatch_process_exit(),
            Lv2Request::ProcessGetPid => self.dispatch_process_get_pid(),
            Lv2Request::ProcessGetPpid => self.dispatch_process_get_ppid(),
            Lv2Request::ProcessGetPpuGuid => self.dispatch_process_get_ppu_guid(),
            Lv2Request::ProcessIsStack { .. } => self.dispatch_process_is_stack(),
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
            Lv2Request::TimerCreate { id_ptr } => {
                self.process_counts.timer_inc();
                let id = self.alloc_id();
                self.immediate_write_u32(id, id_ptr, requester)
            }
            Lv2Request::TimerDestroy { .. } => {
                self.process_counts.timer_dec();
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
            }
            Lv2Request::RwlockCreate { id_ptr, .. } => {
                self.process_counts.rwlock_inc();
                let id = self.alloc_id();
                self.immediate_write_u32(id, id_ptr, requester)
            }
            Lv2Request::RwlockDestroy { .. } => {
                self.process_counts.rwlock_dec();
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
            }
            Lv2Request::EventPortCreate { id_ptr, .. } => {
                self.process_counts.event_port_inc();
                let id = self.alloc_id();
                self.immediate_write_u32(id, id_ptr, requester)
            }
            Lv2Request::EventPortDestroy { .. } => {
                self.process_counts.event_port_dec();
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
            }
            Lv2Request::CallbackDispatchSpawn { .. } => {
                // Fabricated internally via `call_guest_callback_sync`;
                // reaching dispatch through `Lv2Request` is a layering
                // bug -- the classifier never decodes this variant.
                self.record_invariant_break(
                    "dispatch.callback_dispatch_spawn_via_request",
                    format_args!(
                        "CallbackDispatchSpawn reached dispatch via Lv2Request; should be \
                         constructed only as Lv2Dispatch::CallbackSpawn from \
                         call_guest_callback_sync"
                    ),
                );
                Lv2Dispatch::Immediate {
                    code: errno::CELL_EINVAL.into(),
                    effects: vec![],
                }
            }
            Lv2Request::CallbackDispatchReturn { args } => {
                self.dispatch_callback_return(requester, args)
            }
            Lv2Request::Hypercall { lev, r11, args } => {
                // PS3 usermode never issues `sc` with LEV != 0;
                // reject with CELL_EINVAL rather than letting the
                // call fall through to LV2.
                self.log_invariant_break(
                    "dispatch.hypercall_rejected",
                    format_args!(
                        "sc LEV={lev} r11={r11:#x} from PS3 usermode; \
                         hypercalls are a programming error \
                         (r3={:#x} r4={:#x} r5={:#x} r6={:#x} r7={:#x} r8={:#x} r9={:#x} r10={:#x})",
                        args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                    ),
                );
                Lv2Dispatch::Immediate {
                    code: errno::CELL_EINVAL.into(),
                    effects: vec![],
                }
            }
            Lv2Request::Unsupported { number, args } => {
                self.log_invariant_break(
                    "dispatch.unsupported_stub",
                    format_args!(
                        "syscall {number} has no dispatch handler (r3={:#x} r4={:#x} r5={:#x} \
                         r6={:#x} r7={:#x} r8={:#x} r9={:#x} r10={:#x}); returning CELL_OK stub \
                         (guests keying on errno for this syscall will misbehave)",
                        args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                    ),
                );
                Lv2Dispatch::Immediate {
                    code: 0u64,
                    effects: vec![],
                }
            }
            Lv2Request::Malformed {
                number,
                reason,
                args,
            } => {
                self.log_invariant_break(
                    "dispatch.malformed_syscall",
                    format_args!(
                        "syscall {number} rejected: {reason} (r3={:#x} r4={:#x} r5={:#x} \
                         r6={:#x} r7={:#x} r8={:#x} r9={:#x} r10={:#x})",
                        args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                    ),
                );
                Lv2Dispatch::Immediate {
                    code: errno::CELL_EINVAL.into(),
                    effects: vec![],
                }
            }
        }
    }

    /// Append the TTY buffer into [`Self::tty_log`] and write
    /// `nwritten` back. An unmapped buffer skips the append and
    /// still reports `len` written.
    pub(super) fn dispatch_tty_write(
        &mut self,
        buf_ptr: u32,
        len: u32,
        nwritten_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if len > 0 {
            if let Some(bytes) = rt.read_committed(buf_ptr as u64, len as usize) {
                self.tty_log.extend_from_slice(bytes);
            }
        }
        self.immediate_write_u32(len, nwritten_ptr, requester)
    }

    /// Build an immediate dispatch that writes `value` (BE u32) to
    /// `ptr` and returns CELL_OK; shared by create-style syscalls
    /// that emit a freshly allocated id through an out-pointer.
    pub(super) fn immediate_write_u32(&self, value: u32, ptr: u32, source: UnitId) -> Lv2Dispatch {
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(ptr, 4),
            bytes: WritePayload::from_slice(&value.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::test_support::FakeRuntime;

    #[test]
    fn tty_write_writes_nwritten_and_returns_ok() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let req = Lv2Request::TtyWrite {
            fd: 0,
            buf_ptr: 0x8000,
            len: 64,
            nwritten_ptr: 0x9000,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 1);
                if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                    assert_eq!(range.start().raw(), 0x9000);
                    assert_eq!(range.length(), 4);
                    assert_eq!(bytes.bytes(), &64u32.to_be_bytes());
                } else {
                    panic!("expected SharedWriteIntent");
                }
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn tty_write_appends_buffer_bytes_to_tty_log() {
        let mut mem = cellgov_mem::GuestMemory::new(0x10000);
        mem.apply_commit(ByteRange::contiguous_u32(0x8000, 12), b"hello world\n")
            .unwrap();
        let rt = FakeRuntime::with_memory(mem);
        let mut host = Lv2Host::new();
        host.dispatch(
            Lv2Request::TtyWrite {
                fd: 1,
                buf_ptr: 0x8000,
                len: 12,
                nwritten_ptr: 0x9000,
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(host.tty_log(), b"hello world\n");
    }

    #[test]
    fn tty_write_concatenates_across_calls_in_dispatch_order() {
        let mut mem = cellgov_mem::GuestMemory::new(0x10000);
        mem.apply_commit(ByteRange::contiguous_u32(0x8000, 4), b"abcd")
            .unwrap();
        mem.apply_commit(ByteRange::contiguous_u32(0x8100, 3), b"xyz")
            .unwrap();
        let rt = FakeRuntime::with_memory(mem);
        let mut host = Lv2Host::new();
        host.dispatch(
            Lv2Request::TtyWrite {
                fd: 1,
                buf_ptr: 0x8000,
                len: 4,
                nwritten_ptr: 0x9000,
            },
            UnitId::new(0),
            &rt,
        );
        host.dispatch(
            Lv2Request::TtyWrite {
                fd: 1,
                buf_ptr: 0x8100,
                len: 3,
                nwritten_ptr: 0x9000,
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(host.tty_log(), b"abcdxyz");
    }

    #[test]
    fn tty_write_zero_len_is_a_noop_for_tty_log() {
        let rt = FakeRuntime::new(0x10000);
        let mut host = Lv2Host::new();
        host.dispatch(
            Lv2Request::TtyWrite {
                fd: 1,
                buf_ptr: 0x8000,
                len: 0,
                nwritten_ptr: 0x9000,
            },
            UnitId::new(0),
            &rt,
        );
        assert!(host.tty_log().is_empty());
    }

    #[test]
    fn tty_write_unmapped_buf_does_not_corrupt_tty_log_and_still_returns_ok() {
        let rt = FakeRuntime::new(0x1000);
        let mut host = Lv2Host::new();
        let result = host.dispatch(
            Lv2Request::TtyWrite {
                fd: 1,
                buf_ptr: 0x8000,
                len: 4,
                nwritten_ptr: 0x100,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, .. } => assert_eq!(code, 0),
            other => panic!("expected Immediate, got {other:?}"),
        }
        assert!(host.tty_log().is_empty());
    }

    #[test]
    fn time_get_current_time_writes_sec_and_nsec_at_zero_tick() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let result = host.dispatch(
            Lv2Request::TimeGetCurrentTime {
                sec_ptr: 0x8000,
                nsec_ptr: 0x8008,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 2);
                for eff in &effects {
                    if let Effect::SharedWriteIntent { range, bytes, .. } = eff {
                        assert_eq!(range.length(), 8);
                        assert_eq!(bytes.bytes(), &0u64.to_be_bytes());
                    } else {
                        panic!("expected SharedWriteIntent");
                    }
                }
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn time_get_current_time_splits_at_billion_tick() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000).with_tick(cellgov_time::GuestTicks::new(1_500_000_001));
        let result = host.dispatch(
            Lv2Request::TimeGetCurrentTime {
                sec_ptr: 0x1000,
                nsec_ptr: 0x1008,
            },
            UnitId::new(0),
            &rt,
        );
        let effects = match result {
            Lv2Dispatch::Immediate { effects, .. } => effects,
            other => panic!("expected Immediate, got {other:?}"),
        };
        if let Effect::SharedWriteIntent { bytes, .. } = &effects[0] {
            let v = u64::from_be_bytes(bytes.bytes().try_into().unwrap());
            assert_eq!(v, 1);
        } else {
            panic!();
        }
        if let Effect::SharedWriteIntent { bytes, .. } = &effects[1] {
            let v = u64::from_be_bytes(bytes.bytes().try_into().unwrap());
            assert_eq!(v, 500_000_001);
        } else {
            panic!();
        }
    }

    #[test]
    fn time_get_current_time_and_timebase_frequency_are_coherent() {
        let tick_later = 3 * cellgov_time::SIMULATED_INSTRUCTIONS_PER_SECOND + 500_000_000;
        let (sec, nsec) = cellgov_time::ticks_to_sec_nsec(tick_later);
        let tb = cellgov_time::ticks_to_tb(tick_later);
        let as_nsec_from_time_syscall = sec * 1_000_000_000 + nsec;
        let us_from_tb = tb * 1_000_000 / cellgov_time::CELL_PPU_TIMEBASE_HZ;
        let nsec_from_tb = us_from_tb * 1_000;
        // TB granularity is ~12.5 ns; require agreement under 1 us.
        let diff = as_nsec_from_time_syscall.abs_diff(nsec_from_tb);
        assert!(
            diff < 1_000,
            "time syscall and mftb must agree within 1 us: got {diff} ns"
        );
    }

    #[test]
    fn time_get_timebase_frequency_returns_cell_ppu_timebase_hz() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(Lv2Request::TimeGetTimebaseFrequency, UnitId::new(0), &rt);
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: cellgov_time::CELL_PPU_TIMEBASE_HZ,
                effects: vec![],
            }
        );
        assert_eq!(cellgov_time::CELL_PPU_TIMEBASE_HZ, 79_800_000);
    }

    #[test]
    fn cell_ps3_user_memory_total_is_213_mib() {
        // 213 MiB == 0x0D50_0000 == 223,346,688 bytes (PS3 game-mode
        // user-memory cap).
        assert_eq!(cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL, 0x0D50_0000);
        assert_eq!(cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL, 223_346_688);
    }

    #[test]
    fn time_get_timezone_writes_zero_through_both_out_pointers() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::TimeGetTimezone {
                timezone_ptr: 0xd000_fd10,
                summer_time_ptr: 0xd000_fd14,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 2);
                let expected_zero = 0i32.to_be_bytes();
                if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                    assert_eq!(range.start().raw(), 0xd000_fd10);
                    assert_eq!(range.length(), 4);
                    assert_eq!(bytes.bytes(), &expected_zero);
                } else {
                    panic!("expected SharedWriteIntent for timezone_ptr");
                }
                if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[1] {
                    assert_eq!(range.start().raw(), 0xd000_fd14);
                    assert_eq!(range.length(), 4);
                    assert_eq!(bytes.bytes(), &expected_zero);
                } else {
                    panic!("expected SharedWriteIntent for summer_time_ptr");
                }
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn stub_dispatch_returns_cell_ok_for_process_exit() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let req = Lv2Request::ProcessExit { code: 0 };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            }
        );
    }

    #[test]
    fn stub_dispatch_returns_cell_ok_for_unsupported() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let req = Lv2Request::Unsupported {
            number: 999,
            args: [0; 8],
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            }
        );
    }

    #[test]
    fn tty_read_returns_eio() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::Unsupported {
                number: 402,
                args: [0; 8],
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: errno::CELL_EIO.into(),
                effects: vec![],
            }
        );
    }

    #[test]
    fn prx_start_module_returns_einval() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::Unsupported {
                number: 481,
                args: [0; 8],
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: errno::CELL_EINVAL.into(),
                effects: vec![],
            }
        );
    }
}
