# WipEout HD Fury (Sony Liverpool, 2009) HLE Import Inventory

- ELF: `tools/rpcs3/dev_bdvd/BCES00664/PS3_GAME/USRDIR/EBOOT.elf`
- Modules imported: 27
- Functions imported: 332

Classification columns:

- **Name**: NID-DB lookup; `<unknown>` means the NID is not in
  `cellgov_ps3_abi::nid`.
- **Class**: `stub_classification(nid)` from the NID DB.
  `stateful` / `unsafe-to-stub` need real impls; `noop-safe`
  is fine returning 0.
- **CellGov**: `impl` if the NID has dedicated handling in
  `cellgov_core::hle::dispatch_hle` or the HLE-keep list in
  `game::prx::load_firmware_prx`; `stub` otherwise (default
  returns 0).

## sys_fs (15 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x2cb51f0d | cellFsClose                                       | noop-safe       | stub    |
| 0x3f61245c | cellFsOpendir                                     | noop-safe       | stub    |
| 0x4d5ff8e2 | cellFsRead                                        | noop-safe       | stub    |
| 0x5c74903d | cellFsReaddir                                     | noop-safe       | stub    |
| 0x718bf5f8 | cellFsOpen                                        | noop-safe       | stub    |
| 0x7de6dced | cellFsStat                                        | noop-safe       | stub    |
| 0x7f4677a8 | cellFsUnlink                                      | noop-safe       | stub    |
| 0xa397d042 | cellFsLseek                                       | noop-safe       | stub    |
| 0xaa3b4bcd | cellFsGetFreeSize                                 | noop-safe       | stub    |
| 0xba901fe6 | cellFsMkdir                                       | noop-safe       | stub    |
| 0xecdcf2ab | cellFsWrite                                       | noop-safe       | stub    |
| 0xef3efa34 | cellFsFstat                                       | noop-safe       | stub    |
| 0xff42dcc3 | cellFsClosedir                                    | noop-safe       | stub    |
| 0x2796fdf3 | cellFsRmdir                                       | noop-safe       | stub    |
| 0x967a162b | cellFsFsync                                       | noop-safe       | stub    |

## sysPrxForUser (36 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x1573dc3f | sys_lwmutex_lock                                  | noop-safe       | impl    |
| 0x1bc200f4 | sys_lwmutex_unlock                                | noop-safe       | impl    |
| 0x24a1ea07 | sys_ppu_thread_create                             | noop-safe       | impl    |
| 0x26090058 | sys_prx_load_module                               | noop-safe       | stub    |
| 0x2c847572 | _sys_process_atexitspawn                          | noop-safe       | stub    |
| 0x2f85c0ef | sys_lwmutex_create                                | noop-safe       | impl    |
| 0x350d454e | sys_ppu_thread_get_id                             | noop-safe       | impl    |
| 0x409ad939 | sys_mmapper_free_memory                           | noop-safe       | stub    |
| 0x42b23552 | sys_prx_register_library                          | noop-safe       | stub    |
| 0x4643ba6e | sys_mmapper_unmap_memory                          | noop-safe       | stub    |
| 0x70258515 | sys_mmapper_allocate_memory_from_container        | noop-safe       | stub    |
| 0x744680a2 | sys_initialize_tls                                | stateful        | impl    |
| 0x80fb0c19 | sys_prx_stop_module                               | noop-safe       | stub    |
| 0x8461e528 | sys_time_get_system_time                          | noop-safe       | impl    |
| 0x96328741 | _sys_process_at_Exitspawn                         | noop-safe       | stub    |
| 0x9f18429d | sys_prx_start_module                              | noop-safe       | stub    |
| 0xa2c7ba64 | sys_prx_exitspawn_with_level                      | noop-safe       | impl    |
| 0xaff080a4 | sys_ppu_thread_exit                               | noop-safe       | stub    |
| 0xb257540b | sys_mmapper_allocate_memory                       | noop-safe       | stub    |
| 0xc3476d0c | sys_lwmutex_destroy                               | noop-safe       | impl    |
| 0xd0ea47a7 | sys_prx_unregister_library                        | noop-safe       | stub    |
| 0xdc578057 | sys_mmapper_map_memory                            | noop-safe       | stub    |
| 0xe6f2c1e7 | sys_process_exit                                  | stateful        | impl    |
| 0xf0aece0d | sys_prx_unload_module                             | noop-safe       | stub    |
| 0x1c9a942c | sys_lwcond_destroy                                | noop-safe       | stub    |
| 0x2a6d9d51 | sys_lwcond_wait                                   | noop-safe       | stub    |
| 0x45fe2fce | _sys_spu_printf_initialize                        | noop-safe       | stub    |
| 0x4f7172c9 | sys_process_is_stack                              | noop-safe       | impl    |
| 0x67f9fedb | sys_game_process_exitspawn2                       | noop-safe       | stub    |
| 0xa3e3be68 | sys_ppu_thread_once                               | noop-safe       | stub    |
| 0xaeb78725 | sys_lwmutex_trylock                               | noop-safe       | impl    |
| 0xda0eb71a | sys_lwcond_create                                 | noop-safe       | stub    |
| 0xdd3b27ac | _sys_spu_printf_finalize                          | noop-safe       | stub    |
| 0xe0da8efd | sys_spu_image_close                               | noop-safe       | stub    |
| 0xe9a1bd84 | sys_lwcond_signal_all                             | noop-safe       | stub    |
| 0xef87a695 | sys_lwcond_signal                                 | noop-safe       | stub    |

## cellAudio (10 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x0b168f92 | cellAudioInit                                     | noop-safe       | stub    |
| 0x4129fe2d | cellAudioPortClose                                | noop-safe       | stub    |
| 0x74a66af0 | cellAudioGetPortConfig                            | noop-safe       | stub    |
| 0x89be28f2 | cellAudioPortStart                                | noop-safe       | stub    |
| 0xca5ac370 | cellAudioQuit                                     | noop-safe       | stub    |
| 0xcd7bc431 | cellAudioPortOpen                                 | noop-safe       | stub    |
| 0x377e0cd9 | cellAudioSetNotifyEventQueue                      | noop-safe       | stub    |
| 0x56dfe179 | cellAudioSetPortLevel                             | noop-safe       | stub    |
| 0x5b1e2c73 | cellAudioPortStop                                 | noop-safe       | stub    |
| 0xff3626fd | cellAudioRemoveNotifyEventQueue                   | noop-safe       | stub    |

## cellAdec (7 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x1529e506 | cellAdecDecodeAu                                  | noop-safe       | stub    |
| 0x487b613e | cellAdecStartSeq                                  | noop-safe       | stub    |
| 0x7e4a4a49 | cellAdecQueryAttr                                 | noop-safe       | stub    |
| 0x847d2380 | cellAdecClose                                     | noop-safe       | stub    |
| 0x8b5551a4 | cellAdecOpenEx                                    | noop-safe       | stub    |
| 0x97ff2af1 | cellAdecGetPcm                                    | noop-safe       | stub    |
| 0xbd75f78b | cellAdecGetPcmItem                                | noop-safe       | stub    |

## cellSpurs (37 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x07529113 | cellSpursAttributeSetNamePrefix                   | noop-safe       | stub    |
| 0x16394a4e | _cellSpursTasksetAttributeInitialize              | noop-safe       | stub    |
| 0x22aab31d | cellSpursEventFlagDetachLv2EventQueue             | noop-safe       | stub    |
| 0x373523d4 | cellSpursEventFlagWait                            | noop-safe       | stub    |
| 0x4ac7bae4 | cellSpursEventFlagClear                           | noop-safe       | stub    |
| 0x5ef96465 | _cellSpursEventFlagInitialize                     | noop-safe       | stub    |
| 0x652b70e2 | cellSpursTasksetAttributeSetName                  | noop-safe       | stub    |
| 0x87630976 | cellSpursEventFlagAttachLv2EventQueue             | noop-safe       | stub    |
| 0x95180230 | _cellSpursAttributeInitialize                     | noop-safe       | stub    |
| 0x9f72add3 | cellSpursJoinTaskset                              | noop-safe       | stub    |
| 0xa789e631 | cellSpursShutdownTaskset                          | noop-safe       | stub    |
| 0xa839a4d9 | cellSpursAttributeSetSpuThreadGroupType           | noop-safe       | stub    |
| 0xaa6269a8 | cellSpursInitializeWithAttribute                  | noop-safe       | stub    |
| 0xc10931cb | cellSpursCreateTasksetWithAttribute               | noop-safe       | stub    |
| 0xca4c4600 | cellSpursFinalize                                 | noop-safe       | stub    |
| 0x011ee38b | _cellSpursLFQueueInitialize                       | noop-safe       | stub    |
| 0x1656d49f | cellSpursLFQueueAttachLv2EventQueue               | noop-safe       | stub    |
| 0x182d9890 | cellSpursRequestIdleSpu                           | noop-safe       | stub    |
| 0x1f402f8f | cellSpursGetInfo                                  | noop-safe       | stub    |
| 0x4a5eab63 | cellSpursWorkloadAttributeSetName                 | noop-safe       | stub    |
| 0x4e153e3e | cellSpursGetWorkloadInfo                          | noop-safe       | stub    |
| 0x4e66d483 | cellSpursDetachLv2EventQueue                      | noop-safe       | stub    |
| 0x52cc6c82 | cellSpursCreateTaskset                            | noop-safe       | stub    |
| 0x57e4dec3 | cellSpursRemoveWorkload                           | noop-safe       | stub    |
| 0x5fd43fe4 | cellSpursWaitForWorkloadShutdown                  | noop-safe       | stub    |
| 0x73e06f91 | cellSpursLFQueueDetachLv2EventQueue               | noop-safe       | stub    |
| 0x80a29e27 | cellSpursSetPriorities                            | noop-safe       | stub    |
| 0x8a85674d | _cellSpursLFQueuePushBody                         | noop-safe       | stub    |
| 0x98d5b343 | cellSpursShutdownWorkload                         | noop-safe       | stub    |
| 0xacfc8dbc | cellSpursInitialize                               | noop-safe       | stub    |
| 0xb9bc6207 | cellSpursAttachLv2EventQueue                      | noop-safe       | stub    |
| 0xbeb600ac | cellSpursCreateTask                               | noop-safe       | stub    |
| 0xc0158d8b | cellSpursAddWorkloadWithAttribute                 | noop-safe       | stub    |
| 0xd2e23fa9 | cellSpursSetExceptionEventHandler                 | noop-safe       | stub    |
| 0xe0a6dbe4 | _cellSpursSendSignal                              | noop-safe       | stub    |
| 0xefeb2679 | _cellSpursWorkloadAttributeInitialize             | noop-safe       | stub    |
| 0xf843818d | cellSpursReadyCountStore                          | noop-safe       | stub    |

## cellSysmodule (4 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x112a5ee9 | cellSysmoduleUnloadModule                         | noop-safe       | stub    |
| 0x32267a31 | cellSysmoduleLoadModule                           | noop-safe       | stub    |
| 0x5a59e258 | cellSysmoduleIsLoaded                             | noop-safe       | stub    |
| 0x63ff6ff9 | cellSysmoduleInitialize                           | noop-safe       | stub    |

## cellPhotoUtility (3 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x08cbd8e1 | cellPhotoExportInitialize2                        | noop-safe       | stub    |
| 0x09ce84ac | cellPhotoExportFromFile                           | noop-safe       | stub    |
| 0xed4a0148 | cellPhotoExportFinalize                           | noop-safe       | stub    |

## cellSysutil (32 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x02ff3c1b | cellSysutilUnregisterCallback                     | noop-safe       | stub    |
| 0x189a74da | cellSysutilCheckCallback                          | noop-safe       | stub    |
| 0x1e7bff94 | cellSysCacheMount                                 | noop-safe       | stub    |
| 0x20543730 | cellMsgDialogClose                                | noop-safe       | stub    |
| 0x3e22cb4b | cellMsgDialogOpenErrorCode                        | noop-safe       | stub    |
| 0x40e895d3 | cellSysutilGetSystemParamInt                      | noop-safe       | stub    |
| 0x4692ab35 | cellAudioOutConfigure                             | noop-safe       | stub    |
| 0x744c1544 | cellSysCacheClear                                 | noop-safe       | stub    |
| 0x7603d3db | cellMsgDialogOpen2                                | noop-safe       | stub    |
| 0x8b7ed64b | cellSaveDataAutoSave2                             | noop-safe       | stub    |
| 0x94862702 | cellMsgDialogProgressBarInc                       | noop-safe       | stub    |
| 0x9d6af72a | cellMsgDialogProgressBarSetMsg                    | noop-safe       | stub    |
| 0x9d98afa0 | cellSysutilRegisterCallback                       | noop-safe       | stub    |
| 0xc01b4e7c | cellAudioOutGetSoundAvailability                  | noop-safe       | stub    |
| 0xc96e89e9 | cellAudioOutSetCopyControl                        | noop-safe       | stub    |
| 0xcfdd8e87 | cellSysutilDisableBgmPlayback                     | noop-safe       | stub    |
| 0xf4e3caa0 | cellAudioOutGetState                              | noop-safe       | stub    |
| 0xfbd5c856 | cellSaveDataAutoLoad2                             | noop-safe       | stub    |
| 0x0bae8772 | cellVideoOutConfigure                             | noop-safe       | stub    |
| 0x35beade0 | cellOskDialogGetSize                              | noop-safe       | stub    |
| 0x3d1e1931 | cellOskDialogUnloadAsync                          | noop-safe       | stub    |
| 0x7663e368 | cellAudioOutGetDeviceInfo                         | noop-safe       | stub    |
| 0x7f21c918 | cellOskDialogAddSupportLanguage                   | noop-safe       | stub    |
| 0x7fcfc915 | cellOskDialogLoadAsync                            | noop-safe       | stub    |
| 0x887572d5 | cellVideoOutGetState                              | noop-safe       | stub    |
| 0xa322db75 | cellVideoOutGetResolutionAvailability             | noop-safe       | stub    |
| 0xb53c54fa | cellOskDialogSetKeyLayoutOption                   | noop-safe       | stub    |
| 0xb6d84526 | cellOskDialogAbort                                | noop-safe       | stub    |
| 0xe558748d | cellVideoOutGetResolution                         | noop-safe       | stub    |
| 0xe5e2b09d | cellAudioOutGetNumberOfDevice                     | noop-safe       | stub    |
| 0xed5d96af | cellAudioOutGetConfiguration                      | noop-safe       | stub    |
| 0xf0ec3ccc | cellOskDialogSetLayoutMode                        | noop-safe       | stub    |

## sys_net (23 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x139a9e9b | sys_net_initialize_network_ex                     | noop-safe       | stub    |
| 0x27fb339d | sys_net_if_ctl                                    | noop-safe       | stub    |
| 0xb68d5625 | sys_net_finalize_network                          | noop-safe       | stub    |
| 0xfdb8f926 | sys_net_free_thread_context                       | noop-safe       | stub    |
| 0x051ee3ee | socketpoll                                        | noop-safe       | stub    |
| 0x13efe7f5 | getsockname                                       | noop-safe       | stub    |
| 0x1f953b9f | recvfrom                                          | noop-safe       | stub    |
| 0x28e208bb | listen                                            | noop-safe       | stub    |
| 0x3f09e20a | socketselect                                      | noop-safe       | stub    |
| 0x5a045bd1 | getsockopt                                        | noop-safe       | stub    |
| 0x6005cde1 | _sys_net_errno_loc                                | noop-safe       | stub    |
| 0x64f66d35 | connect                                           | noop-safe       | stub    |
| 0x6db6e8cd | socketclose                                       | noop-safe       | stub    |
| 0x71f4c717 | gethostbyname                                     | noop-safe       | stub    |
| 0x88f03575 | setsockopt                                        | noop-safe       | stub    |
| 0x9647570b | sendto                                            | noop-safe       | stub    |
| 0x9c056962 | socket                                            | noop-safe       | stub    |
| 0xa50777c6 | shutdown                                          | noop-safe       | stub    |
| 0xa9a079e0 | inet_aton                                         | noop-safe       | stub    |
| 0xb0a59804 | bind                                              | noop-safe       | stub    |
| 0xc94f6939 | accept                                            | noop-safe       | stub    |
| 0xdc751b40 | send                                              | noop-safe       | stub    |
| 0xfba04f37 | recv                                              | noop-safe       | stub    |

## sceNp (32 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x01fbbc9b | sceNpBasicSendMessageGui                          | noop-safe       | stub    |
| 0x3f0808aa | sceNpBasicSetPresence                             | noop-safe       | stub    |
| 0x481ce0e8 | sceNpBasicAbortGui                                | noop-safe       | stub    |
| 0x4885aa18 | sceNpTerm                                         | noop-safe       | stub    |
| 0x52a6b523 | sceNpManagerUnregisterCallback                    | noop-safe       | stub    |
| 0x64a704cc | sceNpBasicRecvMessageAttachmentLoad               | noop-safe       | stub    |
| 0x806960ab | sceNpBasicRecvMessageCustom                       | noop-safe       | stub    |
| 0x8297f1ec | sceNpManagerRequestTicket2                        | noop-safe       | stub    |
| 0xa1709abd | sceNpManagerGetEntitlementById                    | noop-safe       | stub    |
| 0xa7bff757 | sceNpManagerGetStatus                             | noop-safe       | stub    |
| 0xad218faf | sceNpDrmIsAvailable                               | noop-safe       | stub    |
| 0xb1e0718b | sceNpManagerGetAccountRegion                      | noop-safe       | stub    |
| 0xbd28fdbf | sceNpInit                                         | noop-safe       | stub    |
| 0xbe07c708 | sceNpManagerGetOnlineId                           | noop-safe       | stub    |
| 0xbe81c71c | sceNpBasicSetPresenceDetails                      | noop-safe       | stub    |
| 0xe7dcd3b4 | sceNpManagerRegisterCallback                      | noop-safe       | stub    |
| 0xeb7a3d84 | sceNpManagerGetChatRestrictionFlag                | noop-safe       | stub    |
| 0xec0a1fbf | sceNpBasicSendMessage                             | noop-safe       | stub    |
| 0xf42c0df8 | sceNpManagerGetOnlineName                         | noop-safe       | stub    |
| 0xfe37a7f4 | sceNpManagerGetNpId                               | noop-safe       | stub    |
| 0x0968aa36 | sceNpManagerGetTicket                             | noop-safe       | stub    |
| 0x27c69eba | sceNpBasicAddFriend                               | noop-safe       | stub    |
| 0x58fa4fcd | sceNpManagerGetTicketParam                        | noop-safe       | stub    |
| 0x73931bd0 | sceNpBasicGetBlockListEntryCount                  | noop-safe       | stub    |
| 0xacb9ee8e | sceNpBasicUnregisterHandler                       | noop-safe       | stub    |
| 0xb66d1c46 | sceNpManagerGetEntitlementIdList                  | noop-safe       | stub    |
| 0xbcc09fe7 | sceNpBasicRegisterHandler                         | noop-safe       | stub    |
| 0xbcdbb2ab | sceNpBasicAddPlayersHistoryAsync                  | noop-safe       | stub    |
| 0xbdc07fd5 | sceNpManagerGetNetworkTime                        | noop-safe       | stub    |
| 0xd208f91d | sceNpUtilCmpNpId                                  | noop-safe       | stub    |
| 0xe035f7d6 | sceNpBasicGetEvent                                | noop-safe       | stub    |
| 0xf2b3338a | sceNpBasicGetBlockListEntry                       | noop-safe       | stub    |

## cellRtc (5 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x2cce9cf5 | cellRtcGetCurrentClockLocalTime                   | noop-safe       | stub    |
| 0x32c941cf | cellRtcGetCurrentClock                            | noop-safe       | stub    |
| 0xc7bdb7eb | cellRtcGetTick                                    | noop-safe       | stub    |
| 0x99b13034 | cellRtcSetTick                                    | noop-safe       | stub    |
| 0xcb90c761 | cellRtcGetTime_t                                  | noop-safe       | stub    |

## cellMic (11 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x1b42101b | cellMicIsAttached                                 | noop-safe       | stub    |
| 0x07e1b12c | cellMicRead                                       | noop-safe       | stub    |
| 0x65336418 | cellMicRemoveNotifyEventQueue                     | noop-safe       | stub    |
| 0x6bc46aab | cellMicReset                                      | noop-safe       | stub    |
| 0x7903400e | cellMicSetNotifyEventQueue                        | noop-safe       | stub    |
| 0x8325e02d | cellMicInit                                       | noop-safe       | stub    |
| 0x8d229f8e | cellMicClose                                      | noop-safe       | stub    |
| 0xc6328caa | cellMicEnd                                        | noop-safe       | stub    |
| 0xdd1b59f0 | cellMicOpen                                       | noop-safe       | stub    |
| 0xdd724314 | cellMicStart                                      | noop-safe       | stub    |
| 0xfcfaf246 | cellMicStop                                       | noop-safe       | stub    |

## sceNpTrophy (5 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x1197b52c | sceNpTrophyRegisterContext                        | noop-safe       | stub    |
| 0x1c25470d | sceNpTrophyCreateHandle                           | noop-safe       | stub    |
| 0x39567781 | sceNpTrophyInit                                   | noop-safe       | stub    |
| 0x8ceedd21 | sceNpTrophyUnlockTrophy                           | noop-safe       | stub    |
| 0xe3bf9a28 | sceNpTrophyCreateContext                          | noop-safe       | stub    |

## sceNpCommerce2 (18 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x104551a6 | sceNpCommerce2DoCheckoutStartAsync                | noop-safe       | stub    |
| 0x2a910f05 | sceNpCommerce2DestroyReq                          | noop-safe       | stub    |
| 0x3539d233 | sceNpCommerce2Init                                | noop-safe       | stub    |
| 0x4d4a094c | sceNpCommerce2Term                                | noop-safe       | stub    |
| 0x62023e98 | sceNpCommerce2CreateSessionAbort                  | noop-safe       | stub    |
| 0x6f67ea80 | sceNpCommerce2DestroyCtx                          | noop-safe       | stub    |
| 0x8df0057f | sceNpCommerce2AbortReq                            | noop-safe       | stub    |
| 0x8f46325b | sceNpCommerce2GetProductInfoStart                 | noop-safe       | stub    |
| 0x91f8843d | sceNpCommerce2CreateSessionFinish                 | noop-safe       | stub    |
| 0xa975ebb4 | sceNpCommerce2GetProductInfoCreateReq             | noop-safe       | stub    |
| 0xbf5f58ea | sceNpCommerce2GetProductInfoGetResult             | noop-safe       | stub    |
| 0xcc18cd2c | sceNpCommerce2CreateSessionStart                  | noop-safe       | stub    |
| 0xd43a130e | sceNpCommerce2DoCheckoutFinishAsync               | noop-safe       | stub    |
| 0xd9fdcec2 | sceNpCommerce2CreateCtx                           | noop-safe       | stub    |
| 0xdb19194c | sceNpCommerce2GetGameSkuInfoFromGameProductInfo   | noop-safe       | stub    |
| 0xef645654 | sceNpCommerce2GetGameProductInfo                  | noop-safe       | stub    |
| 0xef8eafcd | sceNpCommerce2DestroyGetProductInfoResult         | noop-safe       | stub    |
| 0xf798f5e3 | sceNpCommerce2InitGetProductInfoResult            | noop-safe       | stub    |

## cellHttp (4 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x250c386c | cellHttpInit                                      | noop-safe       | stub    |
| 0x522180bc | cellHttpsInit                                     | noop-safe       | stub    |
| 0xd276ff1f | cellHttpEnd                                       | noop-safe       | stub    |
| 0xe6d4202f | cellHttpsEnd                                      | noop-safe       | stub    |

## cellSsl (3 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x1650aea4 | cellSslEnd                                        | noop-safe       | stub    |
| 0x571afaca | cellSslCertificateLoader                          | noop-safe       | stub    |
| 0xfb02c9d2 | cellSslInit                                       | noop-safe       | stub    |

## cellSaveData (1 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x27cb8bc2 | cellSaveDataListDelete                            | noop-safe       | stub    |

## cellSearchUtility (9 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x0a4c8295 | cellSearchStartListSearch                         | noop-safe       | stub    |
| 0x3b210319 | cellSearchGetContentInfoByOffset                  | noop-safe       | stub    |
| 0x64fb0b76 | cellSearchStartContentSearchInList                | noop-safe       | stub    |
| 0x774033d6 | cellSearchEnd                                     | noop-safe       | stub    |
| 0x9663a44b | cellSearchGetContentInfoByContentId               | noop-safe       | stub    |
| 0xbfab7616 | cellSearchFinalize                                | noop-safe       | stub    |
| 0xc81ccf8a | cellSearchInitialize                              | noop-safe       | stub    |
| 0xe73cb0d2 | cellSearchPrepareFile                             | noop-safe       | stub    |
| 0xffb28491 | cellSearchGetContentInfoPath                      | noop-safe       | stub    |

## cellGame (7 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x3a5d726a | cellGameGetParamString                            | noop-safe       | stub    |
| 0x42a2e133 | cellGameCreateGameData                            | noop-safe       | stub    |
| 0x70acec67 | cellGameContentPermit                             | noop-safe       | stub    |
| 0xb0a1f8c6 | cellGameContentErrorDialog                        | noop-safe       | stub    |
| 0xdb9819f3 | cellGameDataCheck                                 | noop-safe       | stub    |
| 0xef9d42d5 | cellGameGetSizeKB                                 | noop-safe       | stub    |
| 0xf52639ea | cellGameBootCheck                                 | noop-safe       | stub    |

## cellJpgDec (7 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x6d9ebccf | cellJpgDecReadHeader                              | noop-safe       | stub    |
| 0x9338a07a | cellJpgDecClose                                   | noop-safe       | stub    |
| 0x976ca5c2 | cellJpgDecOpen                                    | noop-safe       | stub    |
| 0xa7978f59 | cellJpgDecCreate                                  | noop-safe       | stub    |
| 0xaf8bb012 | cellJpgDecDecodeData                              | noop-safe       | stub    |
| 0xd8ea91f8 | cellJpgDecDestroy                                 | noop-safe       | stub    |
| 0xe08f3910 | cellJpgDecSetParameter                            | noop-safe       | stub    |

## DFEngine (1 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x17d490a3 | <unknown>                                         | noop-safe       | stub    |

## cellGameExec (2 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x94e9f81d | cellGameGetHomeLaunchOptionPath                   | noop-safe       | stub    |
| 0xf6acd0bc | cellGameGetBootGameInfo                           | noop-safe       | stub    |

## sys_io (20 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x1cf98800 | cellPadInit                                       | noop-safe       | stub    |
| 0x1f71ecbe | cellKbGetConfiguration                            | noop-safe       | stub    |
| 0x2073b7f6 | cellKbClearBuf                                    | noop-safe       | stub    |
| 0x2f1774d5 | cellKbGetInfo                                     | noop-safe       | stub    |
| 0x3138e632 | cellMouseGetData                                  | noop-safe       | stub    |
| 0x3ef66b95 | cellMouseClearBuf                                 | noop-safe       | stub    |
| 0x433f6ec0 | cellKbInit                                        | noop-safe       | stub    |
| 0x4ab1fa77 | cellKbCnvRawCode                                  | noop-safe       | stub    |
| 0x4d9b75d5 | cellPadEnd                                        | noop-safe       | stub    |
| 0x578e3c98 | cellPadSetPortSetting                             | noop-safe       | stub    |
| 0x5baf30fb | cellMouseGetInfo                                  | noop-safe       | stub    |
| 0x8b72cda1 | cellPadGetData                                    | noop-safe       | stub    |
| 0xa5f85e4d | cellKbSetCodeType                                 | noop-safe       | stub    |
| 0xa703a51d | cellPadGetInfo2                                   | noop-safe       | stub    |
| 0xbfce3285 | cellKbEnd                                         | noop-safe       | stub    |
| 0xc9030138 | cellMouseInit                                     | noop-safe       | stub    |
| 0xdeefdfa7 | cellKbSetReadMode                                 | noop-safe       | stub    |
| 0xe10183ce | cellMouseEnd                                      | noop-safe       | stub    |
| 0xf65544ee | cellPadSetActDirect                               | noop-safe       | stub    |
| 0xff0a21b7 | cellKbRead                                        | noop-safe       | stub    |

## cellKey2char (4 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x14bf2dc1 | cellKey2CharClose                                 | noop-safe       | stub    |
| 0x56776c0d | cellKey2CharGetChar                               | noop-safe       | stub    |
| 0xabf629c1 | cellKey2CharOpen                                  | noop-safe       | stub    |
| 0xbfc03768 | cellKey2CharSetMode                               | noop-safe       | stub    |

## cellGcmSys (26 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x06edea9e | cellGcmSetUserHandler                             | noop-safe       | stub    |
| 0x0a862772 | cellGcmSetQueueHandler                            | noop-safe       | stub    |
| 0x15bae46b | _cellGcmInitBody                                  | noop-safe       | impl    |
| 0x21397818 | _cellGcmSetFlipCommand                            | noop-safe       | stub    |
| 0x21ac3697 | cellGcmAddressToOffset                            | noop-safe       | stub    |
| 0x25b40ab4 | cellGcmSortRemapEaIoAddress                       | noop-safe       | stub    |
| 0x2922aed0 | cellGcmGetOffsetTable                             | noop-safe       | stub    |
| 0x3a33c1fd | _cellGcmFunc15                                    | noop-safe       | stub    |
| 0x4524cccd | cellGcmBindTile                                   | noop-safe       | stub    |
| 0x4ae8d215 | cellGcmSetFlipMode                                | noop-safe       | stub    |
| 0x51c9d62b | cellGcmSetDebugOutputLevel                        | noop-safe       | stub    |
| 0x9dc04436 | cellGcmBindZcull                                  | noop-safe       | stub    |
| 0xa114ec67 | cellGcmMapMainMemory                              | noop-safe       | stub    |
| 0xa41ef7e8 | cellGcmSetFlipHandler                             | noop-safe       | stub    |
| 0xa53d12ae | cellGcmSetDisplayBuffer                           | noop-safe       | stub    |
| 0xa547adde | cellGcmGetControlRegister                         | noop-safe       | impl    |
| 0xa75640e8 | cellGcmUnbindZcull                                | noop-safe       | stub    |
| 0xa91b0402 | cellGcmSetVBlankHandler                           | noop-safe       | stub    |
| 0xacee8542 | cellGcmSetFlipImmediate                           | noop-safe       | stub    |
| 0xbd100dbc | cellGcmSetTileInfo                                | noop-safe       | stub    |
| 0xd01b570d | cellGcmSetGraphicsHandler                         | noop-safe       | stub    |
| 0xd8f88e1a | _cellGcmSetFlipCommandWithWaitLabel               | noop-safe       | stub    |
| 0xd9b7653e | cellGcmUnbindTile                                 | noop-safe       | stub    |
| 0xdb23e867 | cellGcmUnmapIoAddress                             | noop-safe       | stub    |
| 0xe315a0b2 | cellGcmGetConfiguration                           | noop-safe       | impl    |
| 0xf80196c1 | cellGcmGetLabelAddress                            | noop-safe       | impl    |

## cellNetCtl (8 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x04459230 | cellNetCtlNetStartDialogLoadAsync                 | noop-safe       | stub    |
| 0x0ce13c6b | cellNetCtlAddHandler                              | noop-safe       | stub    |
| 0x0f1f13d3 | cellNetCtlNetStartDialogUnloadAsync               | noop-safe       | stub    |
| 0x105ee2cb | cellNetCtlTerm                                    | noop-safe       | stub    |
| 0x1e585b5d | cellNetCtlGetInfo                                 | noop-safe       | stub    |
| 0x8b3eba69 | cellNetCtlGetState                                | noop-safe       | stub    |
| 0x901815c3 | cellNetCtlDelHandler                              | noop-safe       | stub    |
| 0xbd5a59fc | cellNetCtlInit                                    | noop-safe       | stub    |

## cellL10n (2 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0xe6f5711b | UTF16stoUTF8s                                     | noop-safe       | stub    |
| 0xf7681b9a | UTF8stoUTF16s                                     | noop-safe       | stub    |

## Summary

- Total imports: 332
- CellGov-implemented: 16
- Unstubbed stateful (need real impl): 0
- Unstubbed unsafe-to-stub (stub returns wrong value): 0
- Default-stub noop-safe: 316
