# LV2 arm fidelity

<!-- Rendered from `cellgov_lv2::request::fidelity` by
`cargo test -p cellgov_lv2 --test fidelity_doc -- --ignored regenerate`.
Do not edit by hand: the non-ignored test in the same file fails on drift. -->

Of LV2's 1024 syscall slots, 90 classify to a typed arm
and 23 route to a dedicated arm inside `Unsupported`;
the remaining **911 slots are the null backend** --
the honest traced `CELL_ENOSYS` refusal is the default, not
the exception. The `null-backend` rows below are the handful
of syscalls given _dedicated_ routing or diagnostics that
still refuse; the unlisted mass refuses through the generic
arm.

The routing-layer guarantee -- an unhandled syscall returns
`CELL_ENOSYS` through the null backend, never a fabricated
success -- holds for the whole surface. How much real LV2
behavior each _handled_ arm reproduces is a per-arm property.
The tags are reviewed claims: table membership is probe-gated
against dispatch, but a tag's accuracy rests on the review
recorded in each arm's rustdoc, not on a machine check.

| Tag | Meaning |
| --- | --- |
| `modeled` | Full modeled state and ABI, faithful to the oracle. |
| `partial-state` | ABI faithful; some kernel-visible state simplified or omitted. |
| `abi-only` | Plausible return value with little or no backing state. |
| `null-backend` | Honest `CELL_ENOSYS`-class refusal with a logged diagnostic. |

Per-arm rationale lives on each dispatch method's rustdoc in
`crates/cellgov_lv2/src/host/`.

## Typed request arms

| Arm | Fidelity |
| --- | --- |
| `SpuImageOpen` | modeled |
| `SpuImageImport` | modeled |
| `SpuThreadGroupCreate` | modeled |
| `SpuThreadInitialize` | modeled |
| `SpuThreadGroupStart` | modeled |
| `SpuThreadGroupDestroy` | modeled |
| `SpuThreadGroupJoin` | modeled |
| `SpuThreadGroupTerminate` | null-backend |
| `TimeGetCurrentTime` | modeled |
| `TimeGetTimebaseFrequency` | modeled |
| `TimeGetTimezone` | abi-only |
| `TtyWrite` | modeled |
| `SpuThreadWriteMb` | modeled |
| `MutexCreate` | modeled |
| `MutexDestroy` | modeled |
| `MutexLock` | modeled |
| `MutexUnlock` | modeled |
| `MutexTryLock` | modeled |
| `SemaphoreCreate` | modeled |
| `SemaphoreDestroy` | modeled |
| `SemaphoreWait` | modeled |
| `SemaphorePost` | modeled |
| `SemaphoreTryWait` | modeled |
| `SemaphoreGetValue` | modeled |
| `EventQueueCreate` | modeled |
| `EventQueueDestroy` | modeled |
| `EventQueueReceive` | modeled |
| `EventPortSend` | modeled |
| `EventFlagCreate` | modeled |
| `EventFlagDestroy` | modeled |
| `EventFlagWait` | modeled |
| `EventFlagSet` | modeled |
| `EventFlagClear` | modeled |
| `EventFlagTryWait` | modeled |
| `EventFlagCancel` | modeled |
| `EventFlagGet` | modeled |
| `EventQueueTryReceive` | modeled |
| `MemoryAllocate` | partial-state |
| `MemoryFree` | abi-only |
| `MemoryGetUserMemorySize` | partial-state |
| `MemoryContainerCreate` | abi-only |
| `ProcessExit` | modeled |
| `ProcessGetPid` | modeled |
| `ProcessGetNumberOfObject` | partial-state |
| `ProcessGetPpid` | modeled |
| `ProcessGetSdkVersion` | modeled |
| `ProcessGetParamsfo` | partial-state |
| `ProcessGetPpuGuid` | abi-only |
| `TimerCreate` | abi-only |
| `TimerDestroy` | abi-only |
| `RwlockCreate` | abi-only |
| `RwlockDestroy` | abi-only |
| `EventPortCreate` | abi-only |
| `EventPortDestroy` | abi-only |
| `ProcessIsStack` | modeled |
| `ProcessIsSpuLockLineReservationAddress` | partial-state |
| `SpuInitialize` | partial-state |
| `PpuThreadYield` | modeled |
| `PpuThreadStart` | partial-state |
| `PpuThreadExit` | modeled |
| `PpuThreadJoin` | modeled |
| `LwMutexCreate` | modeled |
| `LwMutexDestroy` | modeled |
| `LwMutexLock` | modeled |
| `LwMutexUnlock` | modeled |
| `LwMutexTryLock` | modeled |
| `FsOpen` | modeled |
| `FsClose` | modeled |
| `FsRead` | modeled |
| `FsLseek` | modeled |
| `FsFstat` | modeled |
| `FsStat` | modeled |
| `FsOpendir` | modeled |
| `FsReaddir` | modeled |
| `FsClosedir` | modeled |
| `FsWrite` | partial-state |
| `CondCreate` | modeled |
| `CondDestroy` | modeled |
| `CondWait` | modeled |
| `CondSignal` | modeled |
| `CondSignalAll` | modeled |
| `CondSignalTo` | modeled |
| `PpuThreadCreate` | partial-state |
| `SysRsxMemoryAllocate` | partial-state |
| `SysRsxMemoryFree` | abi-only |
| `SysRsxContextAllocate` | partial-state |
| `SysRsxContextFree` | abi-only |
| `SysRsxContextIomap` | modeled |
| `SysRsxDeviceMap` | partial-state |
| `SysRsxContextAttribute` | partial-state |
| `SsAccessControlEngine` | modeled |

## Routed `Unsupported` arms

Syscalls without a typed variant that still reach a dedicated
arm. Any number not listed dispatches to the null backend.

| Syscall | Name | Fidelity |
| --- | --- | --- |
| 48 | `sys_ppu_thread_get_priority` | modeled |
| 136 | `sys_event_port_connect_local` | modeled |
| 137 | `sys_event_port_disconnect` | modeled |
| 140 | `sys_event_port_connect_ipc` | modeled |
| 324 | `sys_memory_container_create` | abi-only |
| 330 | `sys_mmapper_allocate_address` | partial-state |
| 332 | `sys_mmapper_allocate_shared_memory` | partial-state |
| 334 | `sys_mmapper_map_shared_memory` | partial-state |
| 337 | `sys_mmapper_search_and_map` | partial-state |
| 362 | `sys_mmapper_allocate_shared_memory_from_container` | partial-state |
| 402 | `sys_tty_read` | modeled |
| 462 | `uns_func slot 462 (DEX-only)` | modeled |
| 480 | `_sys_prx_load_module` | partial-state |
| 481 | `_sys_prx_start_module` | partial-state |
| 482 | `_sys_prx_stop_module` | partial-state |
| 483 | `_sys_prx_unload_module` | partial-state |
| 484 | `_sys_prx_register_module` | modeled |
| 486 | `_sys_prx_register_library` | partial-state |
| 494 | `_sys_prx_get_module_list` | partial-state |
| 497 | `_sys_prx_load_module_on_memcontainer` | partial-state |
| 512 | `sys_hid_manager_is_process_permission_root` | modeled |
| 621 | `sys_gamepad_ycon_if` | abi-only |
| 677 | `sys_rsx_attribute` | abi-only |
