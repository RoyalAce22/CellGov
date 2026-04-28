# Super Stardust HD (Housemarque, 2007) HLE Import Inventory

- ELF: `tools/rpcs3/dev_hdd0/game/NPUA80068/USRDIR/EBOOT.elf`
- Modules imported: 19
- Functions imported: 200

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

## cellResc (15 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x01220224 | cellRescGcmSurface2RescSrc                        | noop-safe       | stub    |
| 0x0d3c22ce | cellRescSetWaitFlip                               | noop-safe       | stub    |
| 0x10db5b1a | cellRescSetDsts                                   | noop-safe       | stub    |
| 0x129922a0 | cellRescResetFlipStatus                           | noop-safe       | stub    |
| 0x23134710 | cellRescSetDisplayMode                            | noop-safe       | stub    |
| 0x25c107e6 | cellRescSetConvertAndFlip                         | noop-safe       | stub    |
| 0x2ea3061e | cellRescExit                                      | noop-safe       | stub    |
| 0x2ea94661 | cellRescSetFlipHandler                            | noop-safe       | stub    |
| 0x516ee89e | cellRescInit                                      | noop-safe       | stub    |
| 0x5a338cdb | cellRescGetBufferSize                             | noop-safe       | stub    |
| 0x6cd0f95f | cellRescSetSrc                                    | noop-safe       | stub    |
| 0x8107277c | cellRescSetBufferAddress                          | noop-safe       | stub    |
| 0xc47c5c22 | cellRescGetFlipStatus                             | noop-safe       | stub    |
| 0xd3758645 | cellRescSetVBlankHandler                          | noop-safe       | stub    |
| 0xe0cef79e | cellRescCreateInterlaceTable                      | noop-safe       | stub    |

## cellSysmodule (4 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x112a5ee9 | cellSysmoduleUnloadModule                         | noop-safe       | stub    |
| 0x32267a31 | cellSysmoduleLoadModule                           | noop-safe       | stub    |
| 0x63ff6ff9 | cellSysmoduleInitialize                           | noop-safe       | stub    |
| 0x96c07adf | cellSysmoduleFinalize                             | noop-safe       | stub    |

## cellGcmSys (27 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x055bd74d | cellGcmGetTiledPitchSize                          | noop-safe       | impl    |
| 0x06edea9e | cellGcmSetUserHandler                             | noop-safe       | stub    |
| 0x0e6b0dae | cellGcmGetDisplayInfo                             | noop-safe       | stub    |
| 0x15bae46b | _cellGcmInitBody                                  | noop-safe       | impl    |
| 0x21397818 | _cellGcmSetFlipCommand                            | noop-safe       | stub    |
| 0x21ac3697 | cellGcmAddressToOffset                            | noop-safe       | stub    |
| 0x3a33c1fd | _cellGcmFunc15                                    | noop-safe       | stub    |
| 0x4524cccd | cellGcmBindTile                                   | noop-safe       | stub    |
| 0x4ae8d215 | cellGcmSetFlipMode                                | noop-safe       | stub    |
| 0x5e2ee0f0 | cellGcmGetDefaultCommandWordSize                  | noop-safe       | stub    |
| 0x72a577ce | cellGcmGetFlipStatus                              | noop-safe       | stub    |
| 0x8cdf8c70 | cellGcmGetDefaultSegmentWordSize                  | noop-safe       | stub    |
| 0x9ba451e4 | cellGcmSetDefaultFifoSize                         | noop-safe       | stub    |
| 0xa41ef7e8 | cellGcmSetFlipHandler                             | noop-safe       | stub    |
| 0xa53d12ae | cellGcmSetDisplayBuffer                           | noop-safe       | stub    |
| 0xa547adde | cellGcmGetControlRegister                         | noop-safe       | impl    |
| 0xa91b0402 | cellGcmSetVBlankHandler                           | noop-safe       | stub    |
| 0xb2e761d4 | cellGcmResetFlipStatus                            | noop-safe       | stub    |
| 0xbc982946 | cellGcmSetDefaultCommandBuffer                    | noop-safe       | stub    |
| 0xbd100dbc | cellGcmSetTileInfo                                | noop-safe       | stub    |
| 0xd01b570d | cellGcmSetGraphicsHandler                         | noop-safe       | stub    |
| 0xd34a420d | cellGcmSetZcull                                   | noop-safe       | stub    |
| 0xd8f88e1a | _cellGcmSetFlipCommandWithWaitLabel               | noop-safe       | stub    |
| 0xd9b7653e | cellGcmUnbindTile                                 | noop-safe       | stub    |
| 0xe315a0b2 | cellGcmGetConfiguration                           | noop-safe       | impl    |
| 0xf80196c1 | cellGcmGetLabelAddress                            | noop-safe       | impl    |
| 0xffe0160e | cellGcmSetVBlankFrequency                         | noop-safe       | stub    |

## sysPrxForUser (16 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x1573dc3f | sys_lwmutex_lock                                  | noop-safe       | impl    |
| 0x1bc200f4 | sys_lwmutex_unlock                                | noop-safe       | impl    |
| 0x24a1ea07 | sys_ppu_thread_create                             | noop-safe       | impl    |
| 0x2c847572 | _sys_process_atexitspawn                          | noop-safe       | stub    |
| 0x2f85c0ef | sys_lwmutex_create                                | noop-safe       | impl    |
| 0x350d454e | sys_ppu_thread_get_id                             | noop-safe       | impl    |
| 0x42b23552 | sys_prx_register_library                          | noop-safe       | stub    |
| 0x744680a2 | sys_initialize_tls                                | stateful        | impl    |
| 0x8461e528 | sys_time_get_system_time                          | noop-safe       | impl    |
| 0x96328741 | _sys_process_at_Exitspawn                         | noop-safe       | stub    |
| 0xa2c7ba64 | sys_prx_exitspawn_with_level                      | noop-safe       | impl    |
| 0xaff080a4 | sys_ppu_thread_exit                               | noop-safe       | stub    |
| 0xc3476d0c | sys_lwmutex_destroy                               | noop-safe       | impl    |
| 0xe0da8efd | sys_spu_image_close                               | noop-safe       | stub    |
| 0xe6f2c1e7 | sys_process_exit                                  | stateful        | impl    |
| 0xebe5f72f | sys_spu_image_import                              | noop-safe       | stub    |

## sys_fs (8 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x2cb51f0d | cellFsClose                                       | noop-safe       | stub    |
| 0x4d5ff8e2 | cellFsRead                                        | noop-safe       | stub    |
| 0x718bf5f8 | cellFsOpen                                        | noop-safe       | stub    |
| 0x7de6dced | cellFsStat                                        | noop-safe       | stub    |
| 0xa397d042 | cellFsLseek                                       | noop-safe       | stub    |
| 0xba901fe6 | cellFsMkdir                                       | noop-safe       | stub    |
| 0xecdcf2ab | cellFsWrite                                       | noop-safe       | stub    |
| 0xef3efa34 | cellFsFstat                                       | noop-safe       | stub    |

## sys_io (12 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x1cf98800 | cellPadInit                                       | noop-safe       | stub    |
| 0x2073b7f6 | cellKbClearBuf                                    | noop-safe       | stub    |
| 0x433f6ec0 | cellKbInit                                        | noop-safe       | stub    |
| 0x4d9b75d5 | cellPadEnd                                        | noop-safe       | stub    |
| 0x578e3c98 | cellPadSetPortSetting                             | noop-safe       | stub    |
| 0x8b72cda1 | cellPadGetData                                    | noop-safe       | stub    |
| 0xa5f85e4d | cellKbSetCodeType                                 | noop-safe       | stub    |
| 0xa703a51d | cellPadGetInfo2                                   | noop-safe       | stub    |
| 0xbfce3285 | cellKbEnd                                         | noop-safe       | stub    |
| 0xdeefdfa7 | cellKbSetReadMode                                 | noop-safe       | stub    |
| 0xf65544ee | cellPadSetActDirect                               | noop-safe       | stub    |
| 0xff0a21b7 | cellKbRead                                        | noop-safe       | stub    |

## cellSysutil (18 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x02ff3c1b | cellSysutilUnregisterCallback                     | noop-safe       | stub    |
| 0x0bae8772 | cellVideoOutConfigure                             | noop-safe       | stub    |
| 0x189a74da | cellSysutilCheckCallback                          | noop-safe       | stub    |
| 0x220894e3 | cellSysutilEnableBgmPlayback                      | noop-safe       | stub    |
| 0x3e22cb4b | cellMsgDialogOpenErrorCode                        | noop-safe       | stub    |
| 0x40e895d3 | cellSysutilGetSystemParamInt                      | noop-safe       | stub    |
| 0x4692ab35 | cellAudioOutConfigure                             | noop-safe       | stub    |
| 0x75bbb672 | cellVideoOutGetNumberOfDevice                     | noop-safe       | stub    |
| 0x7603d3db | cellMsgDialogOpen2                                | noop-safe       | stub    |
| 0x887572d5 | cellVideoOutGetState                              | noop-safe       | stub    |
| 0x8b7ed64b | cellSaveDataAutoSave2                             | noop-safe       | stub    |
| 0x9d98afa0 | cellSysutilRegisterCallback                       | noop-safe       | stub    |
| 0xa11552f6 | cellSysutilGetBgmPlaybackStatus                   | noop-safe       | stub    |
| 0xa322db75 | cellVideoOutGetResolutionAvailability             | noop-safe       | stub    |
| 0xc01b4e7c | cellAudioOutGetSoundAvailability                  | noop-safe       | stub    |
| 0xe558748d | cellVideoOutGetResolution                         | noop-safe       | stub    |
| 0xedadd797 | cellSaveDataDelete2                               | noop-safe       | stub    |
| 0xfbd5c856 | cellSaveDataAutoLoad2                             | noop-safe       | stub    |

## cellGame (2 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x70acec67 | cellGameContentPermit                             | noop-safe       | stub    |
| 0xf52639ea | cellGameBootCheck                                 | noop-safe       | stub    |

## sceNp (26 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x04372385 | sceNpBasicGetFriendListEntry                      | noop-safe       | stub    |
| 0x05d65dff | sceNpScoreGetRankingByNpId                        | noop-safe       | stub    |
| 0x1672170e | sceNpScoreRecordScore                             | noop-safe       | stub    |
| 0x259113b8 | sceNpScoreDestroyTitleCtx                         | noop-safe       | stub    |
| 0x32cf311f | sceNpScoreInit                                    | noop-safe       | stub    |
| 0x4885aa18 | sceNpTerm                                         | noop-safe       | stub    |
| 0x52a6b523 | sceNpManagerUnregisterCallback                    | noop-safe       | stub    |
| 0x6ee62ed2 | sceNpManagerGetContentRatingFlag                  | noop-safe       | stub    |
| 0x6f5e8143 | sceNpScoreCreateTransactionCtx                    | noop-safe       | stub    |
| 0x8297f1ec | sceNpManagerRequestTicket2                        | noop-safe       | stub    |
| 0x9851f805 | sceNpScoreTerm                                    | noop-safe       | stub    |
| 0xa1709abd | sceNpManagerGetEntitlementById                    | noop-safe       | stub    |
| 0xa7bff757 | sceNpManagerGetStatus                             | noop-safe       | stub    |
| 0xacb9ee8e | sceNpBasicUnregisterHandler                       | noop-safe       | stub    |
| 0xad218faf | sceNpDrmIsAvailable                               | noop-safe       | stub    |
| 0xafef640d | sceNpBasicGetFriendListEntryCount                 | noop-safe       | stub    |
| 0xb1e0718b | sceNpManagerGetAccountRegion                      | noop-safe       | stub    |
| 0xb66d1c46 | sceNpManagerGetEntitlementIdList                  | noop-safe       | stub    |
| 0xb9f93bbb | sceNpScoreCreateTitleCtx                          | noop-safe       | stub    |
| 0xbd28fdbf | sceNpInit                                         | noop-safe       | stub    |
| 0xbe07c708 | sceNpManagerGetOnlineId                           | noop-safe       | stub    |
| 0xc5f4cf82 | sceNpScoreDestroyTransactionCtx                   | noop-safe       | stub    |
| 0xe7dcd3b4 | sceNpManagerRegisterCallback                      | noop-safe       | stub    |
| 0xee5b20d9 | sceNpScoreAbortTransaction                        | noop-safe       | stub    |
| 0xfbc82301 | sceNpScoreGetRankingByRange                       | noop-safe       | stub    |
| 0xfe37a7f4 | sceNpManagerGetNpId                               | noop-safe       | stub    |

## sceNpTrophy (9 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x1197b52c | sceNpTrophyRegisterContext                        | noop-safe       | stub    |
| 0x1c25470d | sceNpTrophyCreateHandle                           | noop-safe       | stub    |
| 0x370136fe | sceNpTrophyGetRequiredDiskSpace                   | noop-safe       | stub    |
| 0x3741ecc7 | sceNpTrophyDestroyContext                         | noop-safe       | stub    |
| 0x39567781 | sceNpTrophyInit                                   | noop-safe       | stub    |
| 0x623cd2dc | sceNpTrophyDestroyHandle                          | noop-safe       | stub    |
| 0x8ceedd21 | sceNpTrophyUnlockTrophy                           | noop-safe       | stub    |
| 0xa7fabf4d | sceNpTrophyTerm                                   | noop-safe       | stub    |
| 0xe3bf9a28 | sceNpTrophyCreateContext                          | noop-safe       | stub    |

## cellSysutilAvconfExt (1 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0xfaa275a4 | cellVideoOutGetScreenSize                         | noop-safe       | stub    |

## cellNetCtl (6 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x04459230 | cellNetCtlNetStartDialogLoadAsync                 | noop-safe       | stub    |
| 0x0f1f13d3 | cellNetCtlNetStartDialogUnloadAsync               | noop-safe       | stub    |
| 0x105ee2cb | cellNetCtlTerm                                    | noop-safe       | stub    |
| 0x1e585b5d | cellNetCtlGetInfo                                 | noop-safe       | stub    |
| 0x8b3eba69 | cellNetCtlGetState                                | noop-safe       | stub    |
| 0xbd5a59fc | cellNetCtlInit                                    | noop-safe       | stub    |

## cellSpurs (19 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x011ee38b | _cellSpursLFQueueInitialize                       | noop-safe       | stub    |
| 0x16394a4e | _cellSpursTasksetAttributeInitialize              | noop-safe       | stub    |
| 0x1656d49f | cellSpursLFQueueAttachLv2EventQueue               | noop-safe       | stub    |
| 0x22aab31d | cellSpursEventFlagDetachLv2EventQueue             | noop-safe       | stub    |
| 0x373523d4 | cellSpursEventFlagWait                            | noop-safe       | stub    |
| 0x4e66d483 | cellSpursDetachLv2EventQueue                      | noop-safe       | stub    |
| 0x5ef96465 | _cellSpursEventFlagInitialize                     | noop-safe       | stub    |
| 0x652b70e2 | cellSpursTasksetAttributeSetName                  | noop-safe       | stub    |
| 0x73e06f91 | cellSpursLFQueueDetachLv2EventQueue               | noop-safe       | stub    |
| 0x87630976 | cellSpursEventFlagAttachLv2EventQueue             | noop-safe       | stub    |
| 0x8a85674d | _cellSpursLFQueuePushBody                         | noop-safe       | stub    |
| 0x8f122ef8 | cellSpursTasksetAttributeSetTasksetSize           | noop-safe       | stub    |
| 0x9f72add3 | cellSpursJoinTaskset                              | noop-safe       | stub    |
| 0xa789e631 | cellSpursShutdownTaskset                          | noop-safe       | stub    |
| 0xacfc8dbc | cellSpursInitialize                               | noop-safe       | stub    |
| 0xb9bc6207 | cellSpursAttachLv2EventQueue                      | noop-safe       | stub    |
| 0xbeb600ac | cellSpursCreateTask                               | noop-safe       | stub    |
| 0xc10931cb | cellSpursCreateTasksetWithAttribute               | noop-safe       | stub    |
| 0xca4c4600 | cellSpursFinalize                                 | noop-safe       | stub    |

## cellAudio (7 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x0b168f92 | cellAudioInit                                     | noop-safe       | stub    |
| 0x4129fe2d | cellAudioPortClose                                | noop-safe       | stub    |
| 0x5b1e2c73 | cellAudioPortStop                                 | noop-safe       | stub    |
| 0x74a66af0 | cellAudioGetPortConfig                            | noop-safe       | stub    |
| 0x89be28f2 | cellAudioPortStart                                | noop-safe       | stub    |
| 0xca5ac370 | cellAudioQuit                                     | noop-safe       | stub    |
| 0xcd7bc431 | cellAudioPortOpen                                 | noop-safe       | stub    |

## sys_net (14 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x139a9e9b | sys_net_initialize_network_ex                     | noop-safe       | stub    |
| 0x28e208bb | listen                                            | noop-safe       | stub    |
| 0x3f09e20a | socketselect                                      | noop-safe       | stub    |
| 0x6005cde1 | _sys_net_errno_loc                                | noop-safe       | stub    |
| 0x64f66d35 | connect                                           | noop-safe       | stub    |
| 0x6db6e8cd | socketclose                                       | noop-safe       | stub    |
| 0x88f03575 | setsockopt                                        | noop-safe       | stub    |
| 0x9c056962 | socket                                            | noop-safe       | stub    |
| 0xb0a59804 | bind                                              | noop-safe       | stub    |
| 0xb68d5625 | sys_net_finalize_network                          | noop-safe       | stub    |
| 0xc94f6939 | accept                                            | noop-safe       | stub    |
| 0xdabbc2c0 | <unknown>                                         | noop-safe       | stub    |
| 0xdc751b40 | send                                              | noop-safe       | stub    |
| 0xfba04f37 | recv                                              | noop-safe       | stub    |

## cellHttp (10 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x052a80d9 | cellHttpCreateTransaction                         | noop-safe       | stub    |
| 0x10d0d7fc | cellHttpResponseGetStatusCode                     | noop-safe       | stub    |
| 0x250c386c | cellHttpInit                                      | noop-safe       | stub    |
| 0x32f5cae2 | cellHttpDestroyTransaction                        | noop-safe       | stub    |
| 0x464ff889 | cellHttpResponseGetContentLength                  | noop-safe       | stub    |
| 0x4e4ee53a | cellHttpCreateClient                              | noop-safe       | stub    |
| 0x61c90691 | cellHttpRecvResponse                              | noop-safe       | stub    |
| 0x980855ac | cellHttpDestroyClient                             | noop-safe       | stub    |
| 0xa755b005 | cellHttpSendRequest                               | noop-safe       | stub    |
| 0xd276ff1f | cellHttpEnd                                       | noop-safe       | stub    |

## cellHttpUtil (1 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x32faaf58 | cellHttpUtilParseUri                              | noop-safe       | stub    |

## cellRtc (2 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x99b13034 | cellRtcSetTick                                    | noop-safe       | stub    |
| 0xcb90c761 | cellRtcGetTime_t                                  | noop-safe       | stub    |

## sceNpCommerce2 (3 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x3539d233 | sceNpCommerce2Init                                | noop-safe       | stub    |
| 0x4d4a094c | sceNpCommerce2Term                                | noop-safe       | stub    |
| 0xeef51be0 | sceNpCommerce2ExecuteStoreBrowse                  | noop-safe       | stub    |

## Summary

- Total imports: 200
- CellGov-implemented: 15
- Unstubbed stateful (need real impl): 0
- Unstubbed unsafe-to-stub (stub returns wrong value): 0
- Default-stub noop-safe: 185
