//! NID-to-function-name database for PS3 system libraries.
//!
//! NIDs (Numeric IDs) are 32-bit hashes that identify PS3 system
//! library functions. They are computed as the first 4 bytes
//! (little-endian u32) of SHA-1(function_name + fixed_suffix).
//!
//! This database covers the standard PS3 SDK modules. It is not
//! game-specific -- the same NID always maps to the same function
//! regardless of which game imports it.

/// Look up a function name by its NID.
///
/// Returns `Some((module, function_name))` if the NID is known,
/// `None` otherwise.
pub fn lookup(nid: u32) -> Option<(&'static str, &'static str)> {
    NID_TABLE
        .binary_search_by_key(&nid, |entry| entry.0)
        .ok()
        .map(|i| (NID_TABLE[i].1, NID_TABLE[i].2))
}

/// Classify how safe a HLE stub is for a given NID.
///
/// Returns one of: "stateful" (needs real implementation),
/// "unsafe-to-stub" (stub may cause incorrect behavior),
/// or "noop-safe" (returning 0 is correct).
pub fn stub_classification(nid: u32) -> &'static str {
    match nid {
        0x744680a2 => "stateful",       // sys_initialize_tls
        0xebe5f72f => "unsafe-to-stub", // _sys_malloc
        0x1573dc3f => "stateful",       // _sys_memset
        0xe6f2c1e7 => "stateful",       // sys_process_exit
        _ => "noop-safe",
    }
}

/// (NID, module_name, function_name) -- sorted by NID for binary search.
static NID_TABLE: &[(u32, &str, &str)] = &[
    (
        0x04459230,
        "cellNetCtl",
        "cellNetCtlNetStartDialogLoadAsync",
    ),
    (0x051ee3ee, "sys_net", "socketpoll"),
    (0x055bd74d, "cellGcmSys", "cellGcmGetTiledPitchSize"),
    (0x07254fda, "cellSync", "cellSyncBarrierInitialize"),
    (0x0968aa36, "sceNp", "sceNpManagerGetTicket"),
    (0x0b168f92, "cellAudio", "cellAudioInit"),
    (0x0bae8772, "cellSysutil", "cellVideoOutConfigure"),
    (
        0x0f1f13d3,
        "cellNetCtl",
        "cellNetCtlNetStartDialogUnloadAsync",
    ),
    (0x105ee2cb, "cellNetCtl", "cellNetCtlTerm"),
    (0x112a5ee9, "cellSysmodule", "cellSysmoduleUnloadModule"),
    (0x139a9e9b, "sys_net", "sys_net_initialize_network_ex"),
    (0x13efe7f5, "sys_net", "getsockname"),
    (0x1573dc3f, "sysPrxForUser", "_sys_memset"),
    (0x15bae46b, "cellGcmSys", "_cellGcmInitBody"),
    (0x182d9890, "cellSpurs", "cellSpursRequestIdleSpu"),
    (0x189a74da, "cellSysutil", "cellSysutilCheckCallback"),
    (0x1bc200f4, "sysPrxForUser", "sys_lwmutex_unlock"),
    (0x1cf98800, "sys_io", "cellPadInit"),
    (0x1e585b5d, "cellNetCtl", "cellNetCtlGetInfo"),
    (0x1f402f8f, "cellSpurs", "cellSpursGetInfo"),
    (0x1f953b9f, "sys_net", "recvfrom"),
    (0x2073b7f6, "sys_io", "cellKbClearBuf"),
    (0x21ac3697, "cellGcmSys", "cellGcmAddressToOffset"),
    (0x24a1ea07, "sysPrxForUser", "sys_ppu_thread_create"),
    (0x28e208bb, "sys_net", "listen"),
    (0x2cb51f0d, "sys_fs", "cellFsClose"),
    (0x2f1774d5, "sys_io", "cellKbGetInfo"),
    (0x2f85c0ef, "sysPrxForUser", "sys_lwmutex_create"),
    (0x3138e632, "sys_io", "cellMouseGetData"),
    (0x32267a31, "cellSysmodule", "cellSysmoduleLoadModule"),
    (0x350d454e, "sysPrxForUser", "sys_ppu_thread_get_id"),
    (0x35f21355, "cellSync", "cellSyncBarrierWait"),
    (0x3a33c1fd, "cellGcmSys", "_cellGcmFunc15"),
    (0x3aaad464, "sys_io", "cellPadGetInfo"),
    (0x3f09e20a, "sys_net", "socketselect"),
    (0x3f61245c, "sys_fs", "cellFsOpendir"),
    (0x3f797dff, "sys_io", "cellPadGetRawData"),
    (0x40e895d3, "cellSysutil", "cellSysutilGetSystemParamInt"),
    (0x4129fe2d, "cellAudio", "cellAudioPortClose"),
    (0x433f6ec0, "sys_io", "cellKbInit"),
    (0x4692ab35, "cellSysutil", "cellAudioOutConfigure"),
    (0x4885aa18, "sceNp", "sceNpTerm"),
    (0x4ae8d215, "cellGcmSys", "cellGcmSetFlipMode"),
    (0x4d5ff8e2, "sys_fs", "cellFsRead"),
    (0x4d9b75d5, "sys_io", "cellPadEnd"),
    (0x4e66d483, "cellSpurs", "cellSpursDetachLv2EventQueue"),
    (0x4f7172c9, "sysPrxForUser", "sys_process_is_stack"),
    (0x57e4dec3, "cellSpurs", "cellSpursRemoveWorkload"),
    (0x5a045bd1, "sys_net", "getsockopt"),
    (0x5b1e2c73, "cellAudio", "cellAudioPortStop"),
    (0x5baf30fb, "sys_io", "cellMouseGetInfo"),
    (0x5c74903d, "sys_fs", "cellFsReaddir"),
    (0x5fd43fe4, "cellSpurs", "cellSpursWaitForWorkloadShutdown"),
    (0x6005cde1, "sys_net", "_sys_net_errno_loc"),
    (0x62b0f803, "cellSysutil", "cellMsgDialogAbort"),
    (0x63ff6ff9, "cellSysmodule", "cellSysmoduleInitialize"),
    (0x64f66d35, "sys_net", "connect"),
    (0x69726aa2, "cellSpurs", "cellSpursAddWorkload"),
    (0x6c272124, "cellSync", "cellSyncBarrierTryWait"),
    (0x6db6e8cd, "sys_net", "socketclose"),
    (0x718bf5f8, "sys_fs", "cellFsOpen"),
    (0x71f4c717, "sys_net", "gethostbyname"),
    (0x72a577ce, "cellGcmSys", "cellGcmGetFlipStatus"),
    (0x744680a2, "sysPrxForUser", "sys_initialize_tls"),
    (0x74a66af0, "cellAudio", "cellAudioGetPortConfig"),
    (0x7603d3db, "cellSysutil", "cellMsgDialogOpen2"),
    (0x78200559, "sys_io", "cellPadInfoSensorMode"),
    (0x7e2fef28, "sceNp", "sceNpManagerRequestTicket"),
    (0x7e4ea023, "cellSpurs", "cellSpursWakeUp"),
    (0x7f4677a8, "sys_fs", "cellFsUnlink"),
    (0x80a29e27, "cellSpurs", "cellSpursSetPriorities"),
    (0x8461e528, "sysPrxForUser", "sys_time_get_system_time"),
    (0x887572d5, "cellSysutil", "cellVideoOutGetState"),
    (0x88f03575, "sys_net", "setsockopt"),
    (0x89be28f2, "cellAudio", "cellAudioPortStart"),
    (0x8b72cda1, "sys_io", "cellPadGetData"),
    (0x9117df20, "cellSysutil", "cellHddGameCheck"),
    (0x9647570b, "sys_net", "sendto"),
    (0x96c07adf, "cellSysmodule", "cellSysmoduleFinalize"),
    (0x983fb9aa, "cellGcmSys", "cellGcmSetWaitFlip"),
    (0x98d5b343, "cellSpurs", "cellSpursShutdownWorkload"),
    (0x9c056962, "sys_net", "socket"),
    (0x9d98afa0, "cellSysutil", "cellSysutilRegisterCallback"),
    (0xa114ec67, "cellGcmSys", "cellGcmMapMainMemory"),
    (0xa2c7ba64, "sysPrxForUser", "sys_prx_exitspawn_with_level"),
    (0xa397d042, "sys_fs", "cellFsLseek"),
    (0xa4ed7dfe, "cellSysutil", "cellSaveDataDelete"),
    (0xa50777c6, "sys_net", "shutdown"),
    (0xa53d12ae, "cellGcmSys", "cellGcmSetDisplayBuffer"),
    (0xa547adde, "cellGcmSys", "cellGcmGetControlRegister"),
    (0xa5f85e4d, "sys_io", "cellKbSetCodeType"),
    (0xa7bff757, "sceNp", "sceNpManagerGetStatus"),
    (0xa91b0402, "cellGcmSys", "cellGcmSetVBlankHandler"),
    (0xa9a079e0, "sys_net", "inet_aton"),
    (0xaa3b4bcd, "sys_fs", "cellFsGetFreeSize"),
    (0xacfc8dbc, "cellSpurs", "cellSpursInitialize"),
    (0xaeb78725, "sysPrxForUser", "sys_lwmutex_trylock"),
    (0xaff080a4, "sysPrxForUser", "_sys_heap_create_heap"),
    (0xb0a59804, "sys_net", "bind"),
    (0xb2e761d4, "cellGcmSys", "cellGcmResetFlipStatus"),
    (0xb68d5625, "sys_net", "sys_net_finalize_network"),
    (0xb9bc6207, "cellSpurs", "cellSpursAttachLv2EventQueue"),
    (0xbcc09fe7, "sceNp", "sceNpBasicRegisterHandler"),
    (0xbd28fdbf, "sceNp", "sceNpInit"),
    (0xbd5a59fc, "cellNetCtl", "cellNetCtlInit"),
    (0xbd6d60d9, "cellGcmSys", "cellGcmSetInvalidateTile"),
    (0xbe5be3ba, "sys_io", "cellPadSetSensorMode"),
    (0xbfce3285, "sys_io", "cellKbEnd"),
    (
        0xc01b4e7c,
        "cellSysutil",
        "cellAudioOutGetSoundAvailability",
    ),
    (0xc22c79b5, "cellSysutil", "cellSaveDataAutoLoad"),
    (0xc3476d0c, "sysPrxForUser", "sys_lwmutex_lock"),
    (0xc9030138, "sys_io", "cellMouseInit"),
    (0xc94f6939, "sys_net", "accept"),
    (0xca4c4600, "cellSpurs", "cellSpursFinalize"),
    (0xca5ac370, "cellAudio", "cellAudioQuit"),
    (0xcd7bc431, "cellAudio", "cellAudioPortOpen"),
    (0xd0b1d189, "cellGcmSys", "cellGcmSetTile"),
    (0xd208f91d, "sceNp", "sceNpUtilCmpNpId"),
    (0xd2e23fa9, "cellSpurs", "cellSpursSetExceptionEventHandler"),
    (0xd34a420d, "cellGcmSys", "cellGcmSetZcull"),
    (0xdc09357e, "cellGcmSys", "cellGcmSetFlip"),
    (0xdc751b40, "sys_net", "send"),
    (0xe035f7d6, "sceNp", "sceNpBasicGetEvent"),
    (0xe10183ce, "sys_io", "cellMouseEnd"),
    (0xe315a0b2, "cellGcmSys", "cellGcmGetConfiguration"),
    (0xe558748d, "cellSysutil", "cellVideoOutGetResolution"),
    (0xe6f2c1e7, "sysPrxForUser", "sys_process_exit"),
    (0xe7dcd3b4, "sceNp", "sceNpManagerRegisterCallback"),
    (0xebe5f72f, "sysPrxForUser", "_sys_malloc"),
    (0xecdcf2ab, "sys_fs", "cellFsWrite"),
    (0xef3efa34, "sys_fs", "cellFsFstat"),
    (0xf80196c1, "cellGcmSys", "cellGcmGetLabelAddress"),
    (0xf81eca25, "cellSysutil", "cellMsgDialogOpen"),
    (0xf843818d, "cellSpurs", "cellSpursReadyCountStore"),
    (0xf8a175ec, "cellSysutil", "cellSaveDataAutoSave"),
    (0xfba04f37, "sys_net", "recv"),
    (0xfc52a7a9, "sysPrxForUser", "_sys_free"),
    (0xfe37a7f4, "sceNp", "sceNpManagerGetNpId"),
    (0xff0a21b7, "sys_io", "cellKbRead"),
    (0xff42dcc3, "sys_fs", "cellFsClosedir"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_known_nids() {
        assert_eq!(
            lookup(0x189a74da),
            Some(("cellSysutil", "cellSysutilCheckCallback"))
        );
        assert_eq!(
            lookup(0x744680a2),
            Some(("sysPrxForUser", "sys_initialize_tls"))
        );
        assert_eq!(lookup(0x9c056962), Some(("sys_net", "socket")));
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert_eq!(lookup(0xDEADBEEF), None);
    }

    #[test]
    fn table_is_sorted() {
        for w in NID_TABLE.windows(2) {
            assert!(
                w[0].0 < w[1].0,
                "NID table not sorted: 0x{:08x} >= 0x{:08x}",
                w[0].0,
                w[1].0
            );
        }
    }

    #[test]
    fn stub_classification_known_nids() {
        assert_eq!(stub_classification(0x744680a2), "stateful"); // sys_initialize_tls
        assert_eq!(stub_classification(0xebe5f72f), "unsafe-to-stub"); // _sys_malloc
        assert_eq!(stub_classification(0x1573dc3f), "stateful"); // _sys_memset
        assert_eq!(stub_classification(0xe6f2c1e7), "stateful"); // sys_process_exit
    }

    #[test]
    fn stub_classification_unknown_is_noop_safe() {
        assert_eq!(stub_classification(0xDEADBEEF), "noop-safe");
        assert_eq!(stub_classification(0x00000000), "noop-safe");
    }
}
