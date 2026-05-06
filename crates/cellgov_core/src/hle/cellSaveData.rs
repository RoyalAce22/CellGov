//! `cellSaveData` HLE: AutoLoad / AutoLoad2 dispatch with real
//! `funcStat` callback parking.
//!
//! Save data is never persisted; `isNewData` is hard-coded to YES so
//! AutoLoad always takes the fresh-install branch. AutoSave and the
//! list variants are not implemented.

use cellgov_event::UnitId;
use cellgov_lv2::CallbackReturnStage;
use cellgov_ps3_abi::cell_save_data::{
    cb_result, cb_result_layout, dir_stat_layout, error, is_new_data, set_buf_layout, size,
    stat_get_layout, stat_set_layout,
};
use cellgov_ps3_abi::nid::cell_save_data as save_nid;

use crate::hle::context::{HleContext, HleParkRequest, RuntimeHleAdapter};
use crate::runtime::Runtime;

#[cfg(test)]
pub(crate) const OWNED_NIDS: &[u32] = save_nid::OWNED;

/// Returns `None` when the NID is not owned here.
pub(crate) fn dispatch(
    runtime: &mut Runtime,
    source: UnitId,
    nid: u32,
    args: &[u64; 9],
) -> Option<()> {
    match nid {
        save_nid::AUTO_LOAD => auto_load(&mut adapter(runtime, source, nid), args),
        save_nid::AUTO_LOAD_2 => auto_load_2(&mut adapter(runtime, source, nid), args),
        _ => return None,
    }
    Some(())
}

fn adapter(runtime: &mut Runtime, source: UnitId, nid: u32) -> RuntimeHleAdapter<'_> {
    RuntimeHleAdapter {
        memory: &mut runtime.memory,
        registry: &mut runtime.registry,
        heap_base: runtime.hle.heap_base,
        heap_ptr: &mut runtime.hle.heap_ptr,
        heap_watermark: &mut runtime.hle.heap_watermark,
        heap_warning_mask: &mut runtime.hle.heap_warning_mask,
        next_id: &mut runtime.hle.next_id,
        source,
        nid,
        mutated: false,
        handlers_without_mutation: &mut runtime.hle.handlers_without_mutation,
        pending_callback_spawn: &mut runtime.hle.pending_callback_spawn,
    }
}

/// `cellSaveDataAutoLoad(version, dirName, errDialog, setBuf, funcStat,
/// funcFile, container)`.
///
/// `args[1..=7]` carry r3..=r9. AutoLoad and AutoLoad2 keep separate
/// arms so a future divergence in argument layout cannot conflate them.
fn auto_load(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    auto_load_impl(ctx, args);
}

/// `cellSaveDataAutoLoad2(version, dirName, errDialog, setBuf, funcStat,
/// funcFile, container, userdata)`.
///
/// `args[8]` carries the eighth `userdata` argument absent from AutoLoad.
fn auto_load_2(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    auto_load_impl(ctx, args);
}

/// Shared AutoLoad / AutoLoad2 body.
///
/// # Cross-module contract
///
/// `statGet` is populated to match RPCS3's `savedata_op` `psf.empty()`
/// branch (`rpcs3/Emu/Cell/Modules/cellSaveData.cpp` lines 1502-1523):
///
/// - `hddFreeSizeKB` = 40 GiB - 256 KiB
/// - `isNewData` = YES
/// - `sysSizeKB` = 35 (constant in RPCS3 regardless of state)
/// - `dir.dirName` echoes the title's input `dirName` so a title that
///   debug-prints `statGet` does not fault on a zero string
/// - `fileList` = `setBuf->buf`, giving titles a valid pointer to
///   dereference even when `fileListNum` is zero
///
/// Other fields (`atime`/`mtime`/`ctime`, `getParam.*`, `sizeKB`,
/// `bind`, `fileNum`) rely on the bump allocator's zero-initialized
/// post-condition.
///
/// `args[8]` carries `userdata`; zero for plain AutoLoad.
fn auto_load_impl(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let dir_name = args[2] as u32;
    let set_buf = args[4] as u32;
    let func_stat = args[5] as u32;
    let func_file = args[6] as u32;
    if dir_name == 0 || set_buf == 0 || func_stat == 0 || func_file == 0 {
        ctx.set_return(error::as_r3(error::PARAM));
        return;
    }
    let userdata = args[8] as u32;

    let Some(cb_result_addr) = ctx.heap_alloc(cb_result_layout::SIZE, 16) else {
        ctx.set_return(error::as_r3(error::INTERNAL));
        return;
    };
    let Some(stat_get_addr) = ctx.heap_alloc(stat_get_layout::SIZE, 16) else {
        ctx.set_return(error::as_r3(error::INTERNAL));
        return;
    };
    let Some(stat_set_addr) = ctx.heap_alloc(stat_set_layout::SIZE, 16) else {
        ctx.set_return(error::as_r3(error::INTERNAL));
        return;
    };

    let set_buf_buf = match read_be_u32(ctx, set_buf + set_buf_layout::OFF_BUF) {
        Ok(v) => v,
        Err(()) => {
            ctx.set_return(error::as_r3(error::PARAM));
            return;
        }
    };
    if write_be_u32(
        ctx,
        stat_get_addr + stat_get_layout::OFF_HDD_FREE_SIZE_KB,
        40 * 1024 * 1024 - 256,
    )
    .is_err()
        || write_be_u32(
            ctx,
            stat_get_addr + stat_get_layout::OFF_IS_NEW_DATA,
            is_new_data::YES,
        )
        .is_err()
        || write_be_u32(ctx, stat_get_addr + stat_get_layout::OFF_SYS_SIZE_KB, 35).is_err()
        || write_be_u32(
            ctx,
            stat_get_addr + stat_get_layout::OFF_FILE_LIST,
            set_buf_buf,
        )
        .is_err()
    {
        ctx.set_return(error::as_r3(error::INTERNAL));
        return;
    }
    if copy_truncated_cstr(
        ctx,
        dir_name,
        stat_get_addr + stat_get_layout::OFF_DIR + dir_stat_layout::OFF_DIR_NAME,
        size::DIRNAME,
    )
    .is_err()
    {
        ctx.set_return(error::as_r3(error::PARAM));
        return;
    }

    if userdata != 0
        && write_be_u32(
            ctx,
            cb_result_addr + cb_result_layout::OFF_USERDATA,
            userdata,
        )
        .is_err()
    {
        ctx.set_return(error::as_r3(error::INTERNAL));
        return;
    }

    ctx.park_for_callback(HleParkRequest {
        opd_addr: func_stat,
        args: [
            cb_result_addr as u64,
            stat_get_addr as u64,
            stat_set_addr as u64,
            0,
            0,
            0,
            0,
            0,
        ],
        stage: CallbackReturnStage::AutoLoadAfterStat {
            cb_result_addr,
            stat_get_addr,
            stat_set_addr,
            func_file_opd: func_file,
        },
    });
}

fn write_be_u32(ctx: &mut dyn HleContext, addr: u32, value: u32) -> Result<(), ()> {
    ctx.write_guest(addr as u64, &value.to_be_bytes())
        .map_err(|_| ())
}

fn read_be_u32(ctx: &dyn HleContext, addr: u32) -> Result<u32, ()> {
    let bytes = ctx.read_guest(addr as u64, 4).map_err(|_| ())?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Copy a NUL-terminated string between guest addresses, writing at
/// most `cap-1` non-NUL bytes plus a final NUL.
///
/// Matches RPCS3's `strcpy_trunc` semantics. Relies on `dst` already
/// being zero over `cap` bytes (bump allocator post-condition); only
/// the effective string prefix is written.
fn copy_truncated_cstr(ctx: &mut dyn HleContext, src: u32, dst: u32, cap: u32) -> Result<(), ()> {
    if cap == 0 {
        return Ok(());
    }
    let bytes = ctx.read_guest(src as u64, cap as usize).map_err(|_| ())?;
    let nul_pos = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let effective = nul_pos.min((cap as usize).saturating_sub(1));
    if effective == 0 {
        return Ok(());
    }
    let owned: Vec<u8> = bytes[..effective].to_vec();
    ctx.write_guest(dst as u64, &owned).map_err(|_| ())
}

/// Resume entry for [`CallbackReturnStage::AutoLoadAfterStat`].
///
/// # Cross-module contract
///
/// Reads `cbResult.result` as a big-endian i32 and maps each documented
/// `cb_result` discriminant to the parent's r3:
///
/// - `OK_LAST` -> `CELL_OK`
/// - `OK_NEXT` -> `FAILURE` (funcFile loop is not implemented; an
///   explicit error keeps titles from observing silent success)
/// - `OK_LAST_NOCONFIRM` -> `PARAM` (illegal in `funcStat` per RPCS3
///   `savedata_op` line 1630; only `funcFile`/`funcDone` may use it)
/// - `ERR_NOSPACE` / `ERR_FAILURE` / `ERR_BROKEN` / `ERR_NODATA` ->
///   the matching `cellSaveData` error
/// - any other value (including `ERR_INVALID`) -> `PARAM`
///
/// An unmapped read of the cbResult region maps to `INTERNAL`.
pub(crate) fn resume_after_stat(
    runtime: &mut Runtime,
    waiter: UnitId,
    cb_result_addr: u32,
    _stat_get_addr: u32,
    _stat_set_addr: u32,
    _func_file_opd: u32,
    _args: [u64; 8],
) {
    let result_addr = u64::from(cb_result_addr) + u64::from(cb_result_layout::OFF_RESULT);
    let range = match cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(result_addr), 4) {
        Some(r) => r,
        None => {
            runtime
                .registry
                .set_syscall_return(waiter, error::as_r3(error::INTERNAL));
            return;
        }
    };
    let Ok(bytes) = runtime.memory.read_checked(range) else {
        runtime
            .registry
            .set_syscall_return(waiter, error::as_r3(error::INTERNAL));
        return;
    };
    let result = i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let r3: u64 = match result {
        cb_result::OK_LAST => 0,
        cb_result::OK_NEXT => error::as_r3(error::FAILURE),
        cb_result::OK_LAST_NOCONFIRM => error::as_r3(error::PARAM),
        cb_result::ERR_NOSPACE => error::as_r3(error::NOSPACE),
        cb_result::ERR_FAILURE => error::as_r3(error::FAILURE),
        cb_result::ERR_BROKEN => error::as_r3(error::BROKEN),
        cb_result::ERR_NODATA => error::as_r3(error::NODATA),
        _ => error::as_r3(error::PARAM),
    };
    runtime.registry.set_syscall_return(waiter, r3);
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_event::UnitId;
    use cellgov_exec::{FakeIsaUnit, FakeOp, UnitStatus};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    fn fixture() -> (Runtime, UnitId) {
        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let unit = UnitId::new(0);
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.set_hle_heap_base(0x10000);
        (rt, unit)
    }

    fn args_with_pointers() -> [u64; 9] {
        [
            0,        // syscall index (ignored)
            0,        // version
            0x1_0000, // dirName
            0,        // errDialog
            0x2_0000, // setBuf
            0x3_0000, // funcStat
            0x4_0000, // funcFile
            0,        // container
            0x5_0000, // userdata (AutoLoad2 only)
        ]
    }

    #[test]
    fn auto_load_with_no_save_path_parks_for_func_stat() {
        let (mut rt, unit) = fixture();
        let routed = dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args_with_pointers());
        assert_eq!(routed, Some(()));
        assert_eq!(rt.registry_mut().drain_syscall_return(unit), None);
        let park = rt
            .hle
            .pending_callback_spawn
            .expect("AutoLoad must park for funcStat");
        assert_eq!(park.opd_addr, 0x3_0000);
        let (cb_result_addr, stat_get_addr, stat_set_addr, func_file_opd) = match park.stage {
            CallbackReturnStage::AutoLoadAfterStat {
                cb_result_addr,
                stat_get_addr,
                stat_set_addr,
                func_file_opd,
            } => (cb_result_addr, stat_get_addr, stat_set_addr, func_file_opd),
            other => panic!("expected AutoLoadAfterStat, got {other:?}"),
        };
        assert_ne!(cb_result_addr, 0);
        assert_ne!(stat_get_addr, 0);
        assert_ne!(stat_set_addr, 0);
        assert_eq!(func_file_opd, 0x4_0000);
        assert_eq!(park.args[0], u64::from(cb_result_addr));
        assert_eq!(park.args[1], u64::from(stat_get_addr));
        assert_eq!(park.args[2], u64::from(stat_set_addr));
        assert_eq!(&park.args[3..], &[0u64; 5]);
    }

    #[test]
    fn auto_load_writes_no_save_shape_into_stat_get() {
        let (mut rt, unit) = fixture();
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args_with_pointers());
        let park = rt.hle.pending_callback_spawn.expect("must park");
        let stat_get_addr = match park.stage {
            CallbackReturnStage::AutoLoadAfterStat { stat_get_addr, .. } => stat_get_addr,
            _ => unreachable!(),
        };
        let read_be32 = |off: u32| {
            let mem = rt.memory().as_bytes();
            let a = (stat_get_addr + off) as usize;
            u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]])
        };
        assert_eq!(
            read_be32(stat_get_layout::OFF_HDD_FREE_SIZE_KB),
            40 * 1024 * 1024 - 256,
        );
        assert_eq!(
            read_be32(stat_get_layout::OFF_IS_NEW_DATA),
            is_new_data::YES
        );
        assert_eq!(read_be32(stat_get_layout::OFF_SYS_SIZE_KB), 35);
        assert_eq!(read_be32(stat_get_layout::OFF_BIND), 0);
        assert_eq!(read_be32(stat_get_layout::OFF_SIZE_KB), 0);
        assert_eq!(read_be32(stat_get_layout::OFF_FILE_NUM), 0);
    }

    fn write_guest_cstr(rt: &mut Runtime, addr: u32, s: &[u8]) {
        rt.memory_mut()
            .apply_commit(
                cellgov_mem::ByteRange::new(
                    cellgov_mem::GuestAddr::new(addr as u64),
                    s.len() as u64,
                )
                .unwrap(),
                s,
            )
            .unwrap();
    }

    #[test]
    fn auto_load_echoes_dir_name_into_stat_get() {
        let (mut rt, unit) = fixture();
        let dir_name_addr = 0x1_0000u32;
        write_guest_cstr(&mut rt, dir_name_addr, b"FLOW\0");
        let mut args = args_with_pointers();
        args[2] = u64::from(dir_name_addr);
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args);
        let park = rt.hle.pending_callback_spawn.expect("must park");
        let stat_get_addr = match park.stage {
            CallbackReturnStage::AutoLoadAfterStat { stat_get_addr, .. } => stat_get_addr,
            _ => unreachable!(),
        };
        let dirname_off =
            (stat_get_addr + stat_get_layout::OFF_DIR + dir_stat_layout::OFF_DIR_NAME) as usize;
        let mem = rt.memory().as_bytes();
        assert_eq!(&mem[dirname_off..dirname_off + 4], b"FLOW");
        assert_eq!(mem[dirname_off + 4], 0);
    }

    #[test]
    fn auto_load_truncates_oversize_dir_name() {
        let (mut rt, unit) = fixture();
        let dir_name_addr = 0x1_0000u32;
        let mut s = vec![b'A'; 40];
        s.push(0);
        write_guest_cstr(&mut rt, dir_name_addr, &s);
        let mut args = args_with_pointers();
        args[2] = u64::from(dir_name_addr);
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args);
        let park = rt.hle.pending_callback_spawn.expect("must park");
        let stat_get_addr = match park.stage {
            CallbackReturnStage::AutoLoadAfterStat { stat_get_addr, .. } => stat_get_addr,
            _ => unreachable!(),
        };
        let dirname_off =
            (stat_get_addr + stat_get_layout::OFF_DIR + dir_stat_layout::OFF_DIR_NAME) as usize;
        let mem = rt.memory().as_bytes();
        assert_eq!(&mem[dirname_off..dirname_off + 31], &[b'A'; 31][..]);
        assert_eq!(mem[dirname_off + 31], 0);
    }

    #[test]
    fn auto_load_threads_set_buf_buf_into_file_list() {
        let (mut rt, unit) = fixture();
        let set_buf_addr = 0x2_0000u32;
        rt.memory_mut()
            .apply_commit(
                cellgov_mem::ByteRange::new(
                    cellgov_mem::GuestAddr::new((set_buf_addr + set_buf_layout::OFF_BUF) as u64),
                    4,
                )
                .unwrap(),
                &0xCAFE_C0DEu32.to_be_bytes(),
            )
            .unwrap();
        let mut args = args_with_pointers();
        args[4] = u64::from(set_buf_addr);
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args);
        let park = rt.hle.pending_callback_spawn.expect("must park");
        let stat_get_addr = match park.stage {
            CallbackReturnStage::AutoLoadAfterStat { stat_get_addr, .. } => stat_get_addr,
            _ => unreachable!(),
        };
        let file_list_off = (stat_get_addr + stat_get_layout::OFF_FILE_LIST) as usize;
        let mem = rt.memory().as_bytes();
        let v = u32::from_be_bytes([
            mem[file_list_off],
            mem[file_list_off + 1],
            mem[file_list_off + 2],
            mem[file_list_off + 3],
        ]);
        assert_eq!(v, 0xCAFE_C0DE);
    }

    #[test]
    fn auto_load_2_threads_userdata_into_cb_result() {
        let (mut rt, unit) = fixture();
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD_2, &args_with_pointers());
        let park = rt.hle.pending_callback_spawn.expect("AutoLoad2 must park");
        let cb_result_addr = match park.stage {
            CallbackReturnStage::AutoLoadAfterStat { cb_result_addr, .. } => cb_result_addr,
            _ => unreachable!(),
        };
        let mem = rt.memory().as_bytes();
        let off = (cb_result_addr + cb_result_layout::OFF_USERDATA) as usize;
        let user = u32::from_be_bytes([mem[off], mem[off + 1], mem[off + 2], mem[off + 3]]);
        assert_eq!(user, 0x5_0000);
    }

    #[test]
    fn auto_load_with_null_dir_name_returns_param() {
        let (mut rt, unit) = fixture();
        let mut args = args_with_pointers();
        args[2] = 0;
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args);
        let r3 = rt
            .registry_mut()
            .drain_syscall_return(unit)
            .expect("dispatch sets r3");
        assert_eq!(r3, error::as_r3(error::PARAM));
        assert_eq!(r3, 0xFFFF_FFFF_8002_B404u64);
    }

    #[test]
    fn auto_load_with_null_set_buf_returns_param() {
        let (mut rt, unit) = fixture();
        let mut args = args_with_pointers();
        args[4] = 0;
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args);
        assert_eq!(
            rt.registry_mut().drain_syscall_return(unit),
            Some(error::as_r3(error::PARAM)),
        );
    }

    #[test]
    fn auto_load_with_null_func_stat_returns_param() {
        let (mut rt, unit) = fixture();
        let mut args = args_with_pointers();
        args[5] = 0;
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args);
        assert_eq!(
            rt.registry_mut().drain_syscall_return(unit),
            Some(error::as_r3(error::PARAM)),
        );
    }

    #[test]
    fn auto_load_with_null_func_file_returns_param() {
        let (mut rt, unit) = fixture();
        let mut args = args_with_pointers();
        args[6] = 0;
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args);
        assert_eq!(
            rt.registry_mut().drain_syscall_return(unit),
            Some(error::as_r3(error::PARAM)),
        );
    }

    #[test]
    fn auto_load_marks_handler_as_mutated() {
        let (mut rt, unit) = fixture();
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args_with_pointers());
        let map = rt.hle_handlers_without_mutation();
        assert!(
            !map.contains_key(&save_nid::AUTO_LOAD),
            "AUTO_LOAD wrote r3 via set_return; must not appear in \
             handlers_without_mutation: {map:?}",
        );
    }

    #[test]
    fn auto_load_param_rejection_emits_no_heap_or_park() {
        let (mut rt, unit) = fixture();
        let mut args = args_with_pointers();
        args[2] = 0;
        let heap_before = rt.hle_heap_watermark();
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args);
        assert_eq!(rt.hle_heap_watermark(), heap_before);
        assert_eq!(
            rt.registry().effective_status(unit),
            Some(UnitStatus::Runnable),
        );
        assert!(rt.hle.pending_callback_spawn.is_none());
    }

    #[test]
    fn dispatch_isolates_returns_per_unit() {
        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let unit_a = rt
            .registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        let unit_b = rt
            .registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.set_hle_heap_base(0x10000);

        let mut args_a = args_with_pointers();
        args_a[2] = 0;
        dispatch(&mut rt, unit_a, save_nid::AUTO_LOAD, &args_a);
        assert_eq!(
            rt.registry_mut().drain_syscall_return(unit_a),
            Some(error::as_r3(error::PARAM)),
        );
        assert!(rt.hle.pending_callback_spawn.is_none());

        dispatch(&mut rt, unit_b, save_nid::AUTO_LOAD, &args_with_pointers());
        assert_eq!(rt.registry_mut().drain_syscall_return(unit_b), None);
        assert!(rt.hle.pending_callback_spawn.is_some());
    }

    /// Drive a resume scenario by writing `result` at `OFF_RESULT` and
    /// returning the parent's r3.
    fn drive_resume(result: i32) -> u64 {
        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let unit = rt
            .registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        let cb_result_addr = 0x1_0000u32;
        rt.memory_mut()
            .apply_commit(
                cellgov_mem::ByteRange::new(
                    cellgov_mem::GuestAddr::new(
                        u64::from(cb_result_addr) + u64::from(cb_result_layout::OFF_RESULT),
                    ),
                    4,
                )
                .unwrap(),
                &result.to_be_bytes(),
            )
            .unwrap();
        resume_after_stat(&mut rt, unit, cb_result_addr, 0, 0, 0, [0; 8]);
        rt.registry_mut().drain_syscall_return(unit).unwrap()
    }

    #[test]
    fn resume_after_stat_ok_last_returns_cell_ok() {
        assert_eq!(drive_resume(cb_result::OK_LAST), 0);
    }

    #[test]
    fn resume_after_stat_ok_next_returns_failure_until_func_file_lands() {
        assert_eq!(
            drive_resume(cb_result::OK_NEXT),
            error::as_r3(error::FAILURE)
        );
    }

    #[test]
    fn resume_after_stat_ok_last_noconfirm_is_param() {
        assert_eq!(
            drive_resume(cb_result::OK_LAST_NOCONFIRM),
            error::as_r3(error::PARAM),
        );
    }

    #[test]
    fn resume_after_stat_negative_codes_map_to_save_data_errors() {
        assert_eq!(
            drive_resume(cb_result::ERR_NOSPACE),
            error::as_r3(error::NOSPACE),
        );
        assert_eq!(
            drive_resume(cb_result::ERR_FAILURE),
            error::as_r3(error::FAILURE),
        );
        assert_eq!(
            drive_resume(cb_result::ERR_BROKEN),
            error::as_r3(error::BROKEN),
        );
        assert_eq!(
            drive_resume(cb_result::ERR_NODATA),
            error::as_r3(error::NODATA),
        );
        assert_eq!(
            drive_resume(cb_result::ERR_INVALID),
            error::as_r3(error::PARAM),
        );
    }

    #[test]
    fn resume_after_stat_unknown_result_code_is_param() {
        assert_eq!(drive_resume(0x7FFF_FFFF), error::as_r3(error::PARAM));
        assert_eq!(drive_resume(-100), error::as_r3(error::PARAM));
    }

    #[test]
    fn resume_after_stat_unmapped_cb_result_maps_to_internal() {
        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let unit = rt
            .registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        resume_after_stat(&mut rt, unit, 0xFF00_0000, 0, 0, 0, [0; 8]);
        assert_eq!(
            rt.registry_mut().drain_syscall_return(unit),
            Some(error::as_r3(error::INTERNAL)),
        );
    }

    #[test]
    fn every_cell_save_data_namespace_nid_routes_per_owned() {
        let namespace_nids = &[
            save_nid::AUTO_LOAD,
            save_nid::AUTO_SAVE,
            save_nid::AUTO_LOAD_2,
            save_nid::AUTO_SAVE_2,
            save_nid::LIST_AUTO_LOAD,
        ];
        for &nid in namespace_nids {
            let (mut rt, unit) = fixture();
            let routed = dispatch(&mut rt, unit, nid, &args_with_pointers());
            let owned = OWNED_NIDS.contains(&nid);
            if owned {
                assert_eq!(
                    routed,
                    Some(()),
                    "NID {nid:#010x} is in OWNED_NIDS but dispatch returned None",
                );
            } else {
                assert_eq!(
                    routed, None,
                    "NID {nid:#010x} is NOT in OWNED_NIDS but dispatch claimed it",
                );
            }
        }
    }

    #[test]
    fn synthetic_unrelated_nid_returns_none() {
        let (mut rt, unit) = fixture();
        let routed = dispatch(&mut rt, unit, 0xDEAD_BEEF, &args_with_pointers());
        assert_eq!(routed, None);
    }

    #[test]
    fn owned_nids_match_abi_namespace_owned() {
        let abi_owned: std::collections::BTreeSet<u32> = save_nid::OWNED.iter().copied().collect();
        let dispatcher_owned: std::collections::BTreeSet<u32> =
            OWNED_NIDS.iter().copied().collect();
        assert_eq!(
            dispatcher_owned, abi_owned,
            "cellgov_core::hle::cell_save_data::OWNED_NIDS and \
             cellgov_ps3_abi::nid::cell_save_data::OWNED must agree; \
             dispatcher={dispatcher_owned:?}, abi={abi_owned:?}",
        );
    }
}
