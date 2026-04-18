# flOw (thatgamecompany, 2007) HLE Import Inventory

- ELF: `tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf`
- Modules imported: 12
- Functions imported: 140

Classification columns:

- **Name**: NID-DB lookup; `<unknown>` means the NID is not in
  `cellgov_ppu::nid_db`.
- **Class**: `stub_classification(nid)` from the NID DB.
  `stateful` / `unsafe-to-stub` need real impls; `noop-safe`
  is fine returning 0.
- **CellGov**: `impl` if the NID has dedicated handling in
  `cellgov_core::hle::dispatch_hle` or the HLE-keep list in
  `game::prx::load_firmware_prx`; `stub` otherwise (default
  returns 0).

## cellSysutil (15 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x0bae8772 | cellVideoOutConfigure                             | noop-safe       | stub    |
| 0x189a74da | cellSysutilCheckCallback                          | noop-safe       | stub    |
| 0x40e895d3 | cellSysutilGetSystemParamInt                      | noop-safe       | stub    |
| 0x4692ab35 | cellAudioOutConfigure                             | noop-safe       | stub    |
| 0x62b0f803 | cellMsgDialogAbort                                | noop-safe       | stub    |
| 0x7603d3db | cellMsgDialogOpen2                                | noop-safe       | stub    |
| 0x887572d5 | cellVideoOutGetState                              | noop-safe       | stub    |
| 0x9117df20 | cellHddGameCheck                                  | noop-safe       | stub    |
| 0x9d98afa0 | cellSysutilRegisterCallback                       | noop-safe       | stub    |
| 0xa4ed7dfe | cellSaveDataDelete                                | noop-safe       | stub    |
| 0xc01b4e7c | cellAudioOutGetSoundAvailability                  | noop-safe       | stub    |
| 0xc22c79b5 | cellSaveDataAutoLoad                              | noop-safe       | stub    |
| 0xe558748d | cellVideoOutGetResolution                         | noop-safe       | stub    |
| 0xf81eca25 | cellMsgDialogOpen                                 | noop-safe       | stub    |
| 0xf8a175ec | cellSaveDataAutoSave                              | noop-safe       | stub    |

## sys_net (21 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x051ee3ee | socketpoll                                        | noop-safe       | stub    |
| 0x139a9e9b | sys_net_initialize_network_ex                     | noop-safe       | stub    |
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
| 0xb68d5625 | sys_net_finalize_network                          | noop-safe       | stub    |
| 0xc94f6939 | accept                                            | noop-safe       | stub    |
| 0xdc751b40 | send                                              | noop-safe       | stub    |
| 0xfba04f37 | recv                                              | noop-safe       | stub    |

## cellGcmSys (18 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x055bd74d | cellGcmGetTiledPitchSize                          | noop-safe       | impl    |
| 0x15bae46b | _cellGcmInitBody                                  | noop-safe       | impl    |
| 0x21ac3697 | cellGcmAddressToOffset                            | noop-safe       | stub    |
| 0x3a33c1fd | _cellGcmFunc15                                    | noop-safe       | stub    |
| 0x4ae8d215 | cellGcmSetFlipMode                                | noop-safe       | stub    |
| 0x72a577ce | cellGcmGetFlipStatus                              | noop-safe       | stub    |
| 0x983fb9aa | cellGcmSetWaitFlip                                | noop-safe       | stub    |
| 0xa114ec67 | cellGcmMapMainMemory                              | noop-safe       | stub    |
| 0xa53d12ae | cellGcmSetDisplayBuffer                           | noop-safe       | stub    |
| 0xa547adde | cellGcmGetControlRegister                         | noop-safe       | impl    |
| 0xa91b0402 | cellGcmSetVBlankHandler                           | noop-safe       | stub    |
| 0xb2e761d4 | cellGcmResetFlipStatus                            | noop-safe       | stub    |
| 0xbd6d60d9 | cellGcmSetInvalidateTile                          | noop-safe       | stub    |
| 0xd0b1d189 | cellGcmSetTile                                    | noop-safe       | stub    |
| 0xd34a420d | cellGcmSetZcull                                   | noop-safe       | stub    |
| 0xdc09357e | cellGcmSetFlip                                    | noop-safe       | stub    |
| 0xe315a0b2 | cellGcmGetConfiguration                           | noop-safe       | impl    |
| 0xf80196c1 | cellGcmGetLabelAddress                            | noop-safe       | impl    |

## sys_io (17 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x1cf98800 | cellPadInit                                       | noop-safe       | stub    |
| 0x2073b7f6 | cellKbClearBuf                                    | noop-safe       | stub    |
| 0x2f1774d5 | cellKbGetInfo                                     | noop-safe       | stub    |
| 0x3138e632 | cellMouseGetData                                  | noop-safe       | stub    |
| 0x3aaad464 | cellPadGetInfo                                    | noop-safe       | stub    |
| 0x3f797dff | cellPadGetRawData                                 | noop-safe       | stub    |
| 0x433f6ec0 | cellKbInit                                        | noop-safe       | stub    |
| 0x4d9b75d5 | cellPadEnd                                        | noop-safe       | stub    |
| 0x5baf30fb | cellMouseGetInfo                                  | noop-safe       | stub    |
| 0x78200559 | cellPadInfoSensorMode                             | noop-safe       | stub    |
| 0x8b72cda1 | cellPadGetData                                    | noop-safe       | stub    |
| 0xa5f85e4d | cellKbSetCodeType                                 | noop-safe       | stub    |
| 0xbe5be3ba | cellPadSetSensorMode                              | noop-safe       | stub    |
| 0xbfce3285 | cellKbEnd                                         | noop-safe       | stub    |
| 0xc9030138 | cellMouseInit                                     | noop-safe       | stub    |
| 0xe10183ce | cellMouseEnd                                      | noop-safe       | stub    |
| 0xff0a21b7 | cellKbRead                                        | noop-safe       | stub    |

## cellSysmodule (4 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x112a5ee9 | cellSysmoduleUnloadModule                         | noop-safe       | stub    |
| 0x32267a31 | cellSysmoduleLoadModule                           | noop-safe       | stub    |
| 0x63ff6ff9 | cellSysmoduleInitialize                           | noop-safe       | stub    |
| 0x96c07adf | cellSysmoduleFinalize                             | noop-safe       | stub    |

## cellSpurs (14 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x4e66d483 | cellSpursDetachLv2EventQueue                      | noop-safe       | stub    |
| 0x57e4dec3 | cellSpursRemoveWorkload                           | noop-safe       | stub    |
| 0x5fd43fe4 | cellSpursWaitForWorkloadShutdown                  | noop-safe       | stub    |
| 0x69726aa2 | cellSpursAddWorkload                              | noop-safe       | stub    |
| 0x7e4ea023 | cellSpursWakeUp                                   | noop-safe       | stub    |
| 0x98d5b343 | cellSpursShutdownWorkload                         | noop-safe       | stub    |
| 0xacfc8dbc | cellSpursInitialize                               | noop-safe       | stub    |
| 0xb9bc6207 | cellSpursAttachLv2EventQueue                      | noop-safe       | stub    |
| 0xca4c4600 | cellSpursFinalize                                 | noop-safe       | stub    |
| 0xf843818d | cellSpursReadyCountStore                          | noop-safe       | stub    |
| 0x182d9890 | cellSpursRequestIdleSpu                           | noop-safe       | stub    |
| 0x1f402f8f | cellSpursGetInfo                                  | noop-safe       | stub    |
| 0x80a29e27 | cellSpursSetPriorities                            | noop-safe       | stub    |
| 0xd2e23fa9 | cellSpursSetExceptionEventHandler                 | noop-safe       | stub    |

## cellNetCtl (5 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x04459230 | cellNetCtlNetStartDialogLoadAsync                 | noop-safe       | stub    |
| 0x0f1f13d3 | cellNetCtlNetStartDialogUnloadAsync               | noop-safe       | stub    |
| 0x105ee2cb | cellNetCtlTerm                                    | noop-safe       | stub    |
| 0x1e585b5d | cellNetCtlGetInfo                                 | noop-safe       | stub    |
| 0xbd5a59fc | cellNetCtlInit                                    | noop-safe       | stub    |

## sys_fs (11 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x2cb51f0d | cellFsClose                                       | noop-safe       | stub    |
| 0x3f61245c | cellFsOpendir                                     | noop-safe       | stub    |
| 0x4d5ff8e2 | cellFsRead                                        | noop-safe       | stub    |
| 0x5c74903d | cellFsReaddir                                     | noop-safe       | stub    |
| 0x718bf5f8 | cellFsOpen                                        | noop-safe       | stub    |
| 0x7f4677a8 | cellFsUnlink                                      | noop-safe       | stub    |
| 0xa397d042 | cellFsLseek                                       | noop-safe       | stub    |
| 0xaa3b4bcd | cellFsGetFreeSize                                 | noop-safe       | stub    |
| 0xecdcf2ab | cellFsWrite                                       | noop-safe       | stub    |
| 0xef3efa34 | cellFsFstat                                       | noop-safe       | stub    |
| 0xff42dcc3 | cellFsClosedir                                    | noop-safe       | stub    |

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

## cellSync (3 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x07254fda | cellSyncBarrierInitialize                         | noop-safe       | stub    |
| 0x35f21355 | cellSyncBarrierWait                               | noop-safe       | stub    |
| 0x6c272124 | cellSyncBarrierTryWait                            | noop-safe       | stub    |

## sceNp (10 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x0968aa36 | sceNpManagerGetTicket                             | noop-safe       | stub    |
| 0x4885aa18 | sceNpTerm                                         | noop-safe       | stub    |
| 0x7e2fef28 | sceNpManagerRequestTicket                         | noop-safe       | stub    |
| 0xa7bff757 | sceNpManagerGetStatus                             | noop-safe       | stub    |
| 0xbcc09fe7 | sceNpBasicRegisterHandler                         | noop-safe       | stub    |
| 0xbd28fdbf | sceNpInit                                         | noop-safe       | stub    |
| 0xd208f91d | sceNpUtilCmpNpId                                  | noop-safe       | stub    |
| 0xe035f7d6 | sceNpBasicGetEvent                                | noop-safe       | stub    |
| 0xe7dcd3b4 | sceNpManagerRegisterCallback                      | noop-safe       | stub    |
| 0xfe37a7f4 | sceNpManagerGetNpId                               | noop-safe       | stub    |

## sysPrxForUser (15 functions)

| NID        | Name                                              | Class           | CellGov |
|------------|---------------------------------------------------|-----------------|---------|
| 0x1573dc3f | sys_lwmutex_lock                                  | noop-safe       | impl    |
| 0x1bc200f4 | sys_lwmutex_unlock                                | noop-safe       | impl    |
| 0x24a1ea07 | sys_ppu_thread_create                             | noop-safe       | impl    |
| 0x2f85c0ef | sys_lwmutex_create                                | noop-safe       | impl    |
| 0x350d454e | sys_ppu_thread_get_id                             | noop-safe       | impl    |
| 0x4f7172c9 | sys_process_is_stack                              | noop-safe       | impl    |
| 0x744680a2 | sys_initialize_tls                                | stateful        | impl    |
| 0x8461e528 | sys_time_get_system_time                          | noop-safe       | impl    |
| 0xa2c7ba64 | sys_prx_exitspawn_with_level                      | noop-safe       | impl    |
| 0xaeb78725 | sys_lwmutex_trylock                               | noop-safe       | impl    |
| 0xaff080a4 | sys_ppu_thread_exit                               | noop-safe       | stub    |
| 0xc3476d0c | sys_lwmutex_destroy                               | noop-safe       | impl    |
| 0xe6f2c1e7 | sys_process_exit                                  | stateful        | impl    |
| 0xebe5f72f | sys_spu_image_import                              | noop-safe       | stub    |
| 0xfc52a7a9 | sys_game_process_exitspawn                        | noop-safe       | stub    |

## Summary

- Total imports: 140
- CellGov-implemented: 17
- Unstubbed stateful (need real impl): 0
- Unstubbed unsafe-to-stub (stub returns wrong value): 0
- Default-stub noop-safe: 123
