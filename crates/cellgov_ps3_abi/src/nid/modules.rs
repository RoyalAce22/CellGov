//! The curated `nid_module!` blocks: the NIDs the workspace names at a
//! typed callsite, one module per PS3 library.

/// `sysPrxForUser` NIDs: the user-mode PRX shim that wraps LV2
/// syscalls (TLS init, heap, lwmutex create, time, thread, process).
pub mod sys_prx_for_user {
    crate::nid_module! {
        classified {
            INITIALIZE_TLS = 0x7446_80a2, "sys_initialize_tls";
            PROCESS_EXIT = 0xe6f2_c1e7, "sys_process_exit";
            MALLOC = 0xbdb1_8f83, "_sys_malloc";
            FREE = 0xf7f7_fb20, "_sys_free";
            MEMSET = 0x68b9_b011, "_sys_memset";
            LWMUTEX_CREATE = 0x2f85_c0ef, "sys_lwmutex_create";
            LWMUTEX_DESTROY = 0xc347_6d0c, "sys_lwmutex_destroy";
            LWMUTEX_LOCK = 0x1573_dc3f, "sys_lwmutex_lock";
            LWMUTEX_UNLOCK = 0x1bc2_00f4, "sys_lwmutex_unlock";
            LWMUTEX_TRYLOCK = 0xaeb7_8725, "sys_lwmutex_trylock";
            LWCOND_CREATE = 0xda0e_b71a, "sys_lwcond_create";
            LWCOND_DESTROY = 0x1c9a_942c, "sys_lwcond_destroy";
            HEAP_CREATE_HEAP = 0xb2fc_f2c8, "_sys_heap_create_heap";
            HEAP_DELETE_HEAP = 0xaede_4b03, "_sys_heap_delete_heap";
            HEAP_MALLOC = 0x3516_8520, "_sys_heap_malloc";
            HEAP_MEMALIGN = 0x4426_5c08, "_sys_heap_memalign";
            HEAP_FREE = 0x8a56_1d92, "_sys_heap_free";
            PPU_THREAD_GET_ID = 0x350d_454e, "sys_ppu_thread_get_id";
            PPU_THREAD_CREATE = 0x24a1_ea07, "sys_ppu_thread_create";
            PPU_THREAD_EXIT = 0xaff0_80a4, "sys_ppu_thread_exit";
            PRX_EXITSPAWN_WITH_LEVEL = 0xa2c7_ba64, "sys_prx_exitspawn_with_level";
            TIME_GET_SYSTEM_TIME = 0x8461_e528, "sys_time_get_system_time";
        }
        unclassified {
            PROCESS_IS_STACK = 0x4f71_72c9, "sys_process_is_stack";
        }
    }
}

/// `sys_fs` NIDs. PS3 titles call these PRX wrappers from
/// `libfs.sprx`; the firmware-set loader's GOT patching binds them
/// to the corresponding firmware OPDs at boot, so guest calls land
/// in real firmware code that then issues raw `sys_fs_*` syscalls
/// handled by `Lv2Host::dispatch`.
pub mod sys_fs {
    crate::nid_module! {
        classified {
            OPEN = 0x718b_f5f8, "cellFsOpen";
            READ = 0x4d5f_f8e2, "cellFsRead";
            CLOSE = 0x2cb5_1f0d, "cellFsClose";
            LSEEK = 0xa397_d042, "cellFsLseek";
            FSTAT = 0xef3e_fa34, "cellFsFstat";
            STAT = 0x7de6_dced, "cellFsStat";
            OPENDIR = 0x3f61_245c, "cellFsOpendir";
            READDIR = 0x5c74_903d, "cellFsReaddir";
            CLOSEDIR = 0xff42_dcc3, "cellFsClosedir";
        }
    }
}

/// `cellSysutil` NIDs.
pub mod cell_sysutil {
    crate::nid_module! {
        classified {
            VIDEO_OUT_GET_STATE = 0x8875_72d5, "cellVideoOutGetState";
            VIDEO_OUT_GET_RESOLUTION = 0xe558_748d, "cellVideoOutGetResolution";
        }
    }
}

/// `cellSaveData` NIDs. Firmware publishes them from `libsysutil.prx`
/// (module `cellSysutil_Library`), which exports under the
/// `cellSysutil` namespace. This module groups them by C source file
/// to mirror PSL1GHT's header layout.
///
/// `CLASSIFIED_NIDS` lists only the AutoLoad NODATA fast-path NIDs
/// that have explicit `stub_classification` verdicts. `AUTO_SAVE` /
/// `AUTO_SAVE_2` / `LIST_AUTO_LOAD` are declared in the
/// `unclassified { ... }` block so they reach typed callsites but
/// keep surfacing through the unclaimed-NID log path until a
/// per-NID review lands.
pub mod cell_save_data {
    crate::nid_module! {
        classified {
            AUTO_LOAD = 0xc22c_79b5, "cellSaveDataAutoLoad";
            AUTO_LOAD_2 = 0xfbd5_c856, "cellSaveDataAutoLoad2";
        }
        unclassified {
            AUTO_SAVE = 0xf8a1_75ec, "cellSaveDataAutoSave";
            AUTO_SAVE_2 = 0x8b7e_d64b, "cellSaveDataAutoSave2";
            LIST_AUTO_LOAD = 0x2142_5307, "cellSaveDataListAutoLoad";
        }
    }
}

/// `cellGcmSys` NIDs.
pub mod cell_gcm_sys {
    crate::nid_module! {
        classified {
            GET_TILED_PITCH_SIZE = 0x055b_d74d, "cellGcmGetTiledPitchSize";
            INIT_BODY = 0x15ba_e46b, "_cellGcmInitBody";
            GET_CONFIGURATION = 0xe315_a0b2, "cellGcmGetConfiguration";
            GET_CONTROL_REGISTER = 0xa547_adde, "cellGcmGetControlRegister";
            GET_LABEL_ADDRESS = 0xf801_96c1, "cellGcmGetLabelAddress";
            ADDRESS_TO_OFFSET = 0x21ac_3697, "cellGcmAddressToOffset";
            SET_FLIP_HANDLER = 0xa41e_f7e8, "cellGcmSetFlipHandler";
        }
    }
}

/// `cellSpurs` NIDs (PPU-side surface). Verified at compile time
/// against SHA-1 of the original guest function name via
/// [`crate::nid_const`].
pub mod cell_spurs {
    crate::nid_module! {
        classified {
            ATTRIBUTE_INITIALIZE = 0x9518_0230, "_cellSpursAttributeInitialize";
            INITIALIZE = 0xacfc_8dbc, "cellSpursInitialize";
            INITIALIZE_WITH_ATTRIBUTE = 0xaa62_69a8, "cellSpursInitializeWithAttribute";
            INITIALIZE_WITH_ATTRIBUTE2 = 0x30aa_96c4, "cellSpursInitializeWithAttribute2";
            FINALIZE = 0xca4c_4600, "cellSpursFinalize";
            ADD_WORKLOAD = 0x6972_6aa2, "cellSpursAddWorkload";
            ADD_WORKLOAD_WITH_ATTRIBUTE = 0xc015_8d8b, "cellSpursAddWorkloadWithAttribute";
            WORKLOAD_ATTRIBUTE_INITIALIZE = 0xefeb_2679, "_cellSpursWorkloadAttributeInitialize";
            SHUTDOWN_WORKLOAD = 0x98d5_b343, "cellSpursShutdownWorkload";
            WAIT_FOR_WORKLOAD_SHUTDOWN = 0x5fd4_3fe4, "cellSpursWaitForWorkloadShutdown";
            READY_COUNT_STORE = 0xf843_818d, "cellSpursReadyCountStore";
            READY_COUNT_ADD = 0x7521_1196, "cellSpursReadyCountAdd";
            READY_COUNT_SWAP = 0x49a3_426d, "cellSpursReadyCountSwap";
            READY_COUNT_COMPARE_AND_SWAP = 0xf1d3_552d, "cellSpursReadyCountCompareAndSwap";
            REQUEST_IDLE_SPU = 0x182d_9890, "cellSpursRequestIdleSpu";
            SET_MAX_CONTENTION = 0x84d2_f6d5, "cellSpursSetMaxContention";
            SET_PRIORITIES = 0x80a2_9e27, "cellSpursSetPriorities";
            SET_PRIORITY = 0xb52e_1bda, "cellSpursSetPriority";
            GET_INFO = 0x1f40_2f8f, "cellSpursGetInfo";
            ATTACH_LV2_EVENT_QUEUE = 0xb9bc_6207, "cellSpursAttachLv2EventQueue";
            DETACH_LV2_EVENT_QUEUE = 0x4e66_d483, "cellSpursDetachLv2EventQueue";
            SET_EXCEPTION_EVENT_HANDLER = 0xd2e2_3fa9, "cellSpursSetExceptionEventHandler";
            UNSET_EXCEPTION_EVENT_HANDLER = 0x4c75_deb8, "cellSpursUnsetExceptionEventHandler";
            SET_GLOBAL_EXCEPTION_EVENT_HANDLER = 0x7517_724a, "cellSpursSetGlobalExceptionEventHandler";
            UNSET_GLOBAL_EXCEPTION_EVENT_HANDLER = 0x8612_37f8, "cellSpursUnsetGlobalExceptionEventHandler";
            ENABLE_EXCEPTION_EVENT_HANDLER = 0x32b9_4add, "cellSpursEnableExceptionEventHandler";
        }
    }
}
