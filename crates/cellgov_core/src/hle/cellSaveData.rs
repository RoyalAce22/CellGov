//! cellSaveData HLE implementations.
//!
//! Real callback dispatch via the worker-thread primitive: AutoLoad
//! / AutoLoad2 allocate `cbResult` / `statGet` / `statSet` on the HLE
//! heap, populate `statGet` with the no-save shape (mirrored from
//! RPCS3's `savedata_op` `psf.empty()` branch), then park the calling
//! unit on the title's `funcStat` callback via
//! [`HleContext::park_for_callback`]. On resume, [`resume_after_stat`]
//! reads `cbResult.result` and either finalizes (CELL_OK or a
//! `cellSaveData` error code) or transitions into the funcFile loop
//! (the funcFile path is wired in a follow-on slice; today's
//! resume returns FAILURE on `OK_NEXT`).
//!
//! ## Spec deviations (visible in fault traces)
//!
//! - Save data is never persisted: `isNewData` is hard-coded to
//!   YES. A fresh-install AutoLoad branch is the only successful path.
//! - `dir.atime` / `mtime` / `ctime` and `getParam.*` stay zero
//!   (RPCS3 fills these from filesystem stat / PARAM.SFO; with no
//!   save data, the dir_info struct from a failed stat is still
//!   zero-initialized).
//! - `version`, `errDialog`, and `container` are not validated.
//! - The funcFile loop is not yet exercised; titles whose `funcStat`
//!   returns `OK_NEXT` (drop into funcFile) currently see
//!   `CELL_SAVEDATA_ERROR_FAILURE` instead of running funcFile.
//!
//! ## Anti-scope
//!
//! - No `cellSaveDataAutoSave` / `cellSaveDataAutoSave2`. AutoSave
//!   needs the same primitive but the persistence side is a
//!   successor phase.
//! - No `cellSaveDataListAutoLoad`. The list variant pops a
//!   user-facing dialog; anti-scope for the headless oracle.

use cellgov_event::UnitId;
use cellgov_lv2::CallbackReturnStage;
use cellgov_ps3_abi::cell_save_data::{
    cb_result, cb_result_layout, dir_stat_layout, error, is_new_data, set_buf_layout, size,
    stat_get_layout, stat_set_layout,
};
use cellgov_ps3_abi::nid::cell_save_data as save_nid;

use crate::hle::context::{HleContext, HleParkRequest, RuntimeHleAdapter};
use crate::runtime::Runtime;

/// Every NID this module claims at the dispatcher.
///
/// `cfg(test)` per the workspace pattern (every per-module HLE
/// `OWNED_NIDS` is test-only; the production source of truth is
/// `cellgov_ps3_abi::nid::cell_save_data::OWNED`, flattened into
/// `cellgov_ppu::prx::HLE_IMPLEMENTED_NIDS` via
/// `cellgov_ps3_abi::nid::ALL_HLE_OWNED`). The drift case is caught
/// by the `every_cell_save_data_namespace_nid_routes_per_owned`
/// and `owned_nids_match_abi_namespace_owned` tests below, plus the
/// parent `hle.rs`'s `hle_module_nid_sets_are_disjoint` canary.
#[cfg(test)]
pub(crate) const OWNED_NIDS: &[u32] = save_nid::OWNED;

/// Dispatch entry point; returns `None` if the NID is not owned here.
///
/// AutoLoad and AutoLoad2 share the NODATA fast path but get
/// separate match arms and separate handler functions so the future
/// worker-thread variant cannot conflate their argument layouts
/// (AutoLoad2 has an eighth `userdata` argument that AutoLoad
/// lacks). Today the bodies happen to be identical; the seam stays
/// per-NID by construction.
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

/// `cellSaveDataAutoLoad(version, dirName, errDialog, setBuf,
/// funcStat, funcFile, container)`. Seven args.
///
/// Argument slots in the `[u64; 9]` array (index 0 carries the
/// syscall number, indices 1..=8 carry r3..=r10):
/// - `args[1]` = `version` (u32)
/// - `args[2]` = `dirName` (vm::cptr<char>)
/// - `args[3]` = `errDialog` (u32)
/// - `args[4]` = `setBuf` (vm::ptr<CellSaveDataSetBuf>)
/// - `args[5]` = `funcStat` (vm::ptr<CellSaveDataStatCallback>)
/// - `args[6]` = `funcFile` (vm::ptr<CellSaveDataFileCallback>)
/// - `args[7]` = `container` (u32)
fn auto_load(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    auto_load_impl(ctx, args);
}

/// `cellSaveDataAutoLoad2(version, dirName, errDialog, setBuf,
/// funcStat, funcFile, container, userdata)`. Eight args.
///
/// Same as `cellSaveDataAutoLoad` plus an eighth `userdata` arg.
/// The body shares the AutoLoad pipeline; `userdata` is wired into
/// `cbResult.userdata` so the title's callback observes it.
fn auto_load_2(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    auto_load_impl(ctx, args);
}

/// Shared AutoLoad / AutoLoad2 body.
///
/// Validates the four mandatory pointers, allocates the per-call
/// callback structs on the HLE heap, populates `statGet` with the
/// no-save shape, and parks the calling unit on the title's
/// `funcStat` callback. The resume arm runs in
/// [`resume_after_stat`].
///
/// `args[8]` carries `userdata` for AutoLoad2 (it is zero for
/// plain AutoLoad since the title never wrote to slot 8).
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

    // Allocate cbResult / statGet / statSet on the HLE bump heap.
    // The bump allocator never reuses memory; GuestMemory is zero-
    // initialized at construction; therefore each fresh allocation
    // returns a zero-filled region without an explicit memset.
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

    // Populate statGet with the no-save shape, matching RPCS3's
    // savedata_op output for the `psf.empty()` (isNewData) branch
    // (`rpcs3/Emu/Cell/Modules/cellSaveData.cpp` lines 1502-1523).
    // Fields not written here stay zero (allocator post-condition
    // above): atime/mtime/ctime, getParam.*, sizeKB.
    //
    // - hddFreeSizeKB: 40 GiB - 256 KiB (RPCS3's reported value).
    // - isNewData: YES (no save data).
    // - dir.dirName: echoed from the title's input dirName so the
    //   title's debug-print of statGet sees a populated string.
    // - sysSizeKB: 35 (RPCS3 reports a constant 35 regardless).
    // - fileList: setBuf->buf so titles dereferencing fileList get
    //   a valid (title-supplied) pointer even when fileListNum=0.
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

    // Populate cbResult.userdata for AutoLoad2 (zero for plain
    // AutoLoad, which is the same as the allocator's post-condition).
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

    // Park on funcStat. Worker r3..=r5 carry the three struct
    // pointers; r6..=r10 are zero.
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

/// Helper: write a big-endian u32 at the given guest address.
fn write_be_u32(ctx: &mut dyn HleContext, addr: u32, value: u32) -> Result<(), ()> {
    ctx.write_guest(addr as u64, &value.to_be_bytes())
        .map_err(|_| ())
}

/// Helper: read a big-endian u32 from the given guest address.
fn read_be_u32(ctx: &dyn HleContext, addr: u32) -> Result<u32, ()> {
    let bytes = ctx.read_guest(addr as u64, 4).map_err(|_| ())?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Helper: copy a NUL-terminated string from `src` (guest) to `dst`
/// (guest), truncated to `cap` bytes total including the trailing
/// NUL. Matches RPCS3's `strcpy_trunc` semantics: at most `cap-1`
/// non-NUL bytes plus a final NUL are written. Returns `Err(())`
/// when the source range is unmapped.
///
/// Assumes `dst` is zero-initialized over its full `cap` bytes
/// (the bump allocator's post-condition); only writes up to the
/// effective string length, leaving the trailing zeros intact.
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

/// Resume entry for [`CallbackReturnStage::AutoLoadAfterStat`]:
/// inspect the title's `cbResult.result` and either finalize the
/// AutoLoad call (CELL_OK / `cellSaveData` error) or report
/// `OK_NEXT` as failure (the funcFile loop lands in a follow-on
/// slice).
///
/// Reads four bytes at `cb_result_addr + OFF_RESULT` as a big-endian
/// signed 32-bit integer. An unmapped read (the title's callback
/// somehow corrupted the cb_result region) maps to
/// `CELL_SAVEDATA_ERROR_INTERNAL`.
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
        cb_result::OK_LAST => 0, // CELL_OK
        cb_result::OK_NEXT => {
            // funcFile loop lands in a follow-on slice; until then
            // titles whose funcStat returns OK_NEXT see FAILURE.
            error::as_r3(error::FAILURE)
        }
        // OK_LAST_NOCONFIRM is illegal in funcStat per RPCS3's
        // savedata_op (line 1630); maps to PARAM.
        cb_result::OK_LAST_NOCONFIRM => error::as_r3(error::PARAM),
        cb_result::ERR_NOSPACE => error::as_r3(error::NOSPACE),
        cb_result::ERR_FAILURE => error::as_r3(error::FAILURE),
        cb_result::ERR_BROKEN => error::as_r3(error::BROKEN),
        cb_result::ERR_NODATA => error::as_r3(error::NODATA),
        // ERR_INVALID and any unrecognized result map to PARAM.
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

    /// Minimal runtime for dispatch routing: 1 MiB guest memory,
    /// budget 1, one fake unit. Sufficient because the NODATA stub
    /// never reads guest pointers; the day a slice validates arg
    /// pointers against guest memory, this fixture must grow to
    /// cover mapped vs unmapped ranges.
    fn fixture() -> (Runtime, UnitId) {
        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let unit = UnitId::new(0);
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.set_hle_heap_base(0x10000);
        (rt, unit)
    }

    /// All four mandatory pointers populated; no NULL-rejection
    /// arm fires.
    fn args_with_pointers() -> [u64; 9] {
        // Sentinel guest pointers: any non-zero u32 satisfies the
        // NULL check; the stub never dereferences them.
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

    /// Happy path: AutoLoad parks the calling unit on the title's
    /// funcStat OPD with the three struct addresses wired into
    /// r3..=r5 and the resume stage carrying the same addresses.
    #[test]
    fn auto_load_with_no_save_path_parks_for_func_stat() {
        let (mut rt, unit) = fixture();
        let routed = dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args_with_pointers());
        assert_eq!(routed, Some(()));
        // r3 is set by the resume arm, not at park time.
        assert_eq!(rt.registry_mut().drain_syscall_return(unit), None);
        let park = rt
            .hle
            .pending_callback_spawn
            .expect("AutoLoad must park for funcStat");
        assert_eq!(park.opd_addr, 0x3_0000); // funcStat from args_with_pointers
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
        assert_eq!(func_file_opd, 0x4_0000); // funcFile from args_with_pointers
        assert_eq!(park.args[0], u64::from(cb_result_addr));
        assert_eq!(park.args[1], u64::from(stat_get_addr));
        assert_eq!(park.args[2], u64::from(stat_set_addr));
        assert_eq!(&park.args[3..], &[0u64; 5]);
    }

    /// statGet's no-save shape is written before the title's
    /// funcStat fires: hddFreeSizeKB = 1 GiB, isNewData = YES,
    /// sysSizeKB = 35. Other fields stay zero (the bump allocator
    /// hands out fresh memory which is zero-initialized).
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
        // Matches RPCS3's reported value (40 GiB - 256 KiB) so a
        // future drift here is loud rather than silent.
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

    /// Set up a guest dirName cstring at `addr`; return the
    /// fixture's args mutated to use it. Caller must already have a
    /// runtime with allocated memory at `addr`.
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

    /// `dir.dirName` is populated from the title's input dirName
    /// (NUL-terminated, truncated to 32 bytes). Titles whose
    /// funcStat dump-formatter reads dirName fault on the next
    /// dependent load if this field is left zero.
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
        // Trailing byte is the existing zero from the bump allocator.
        assert_eq!(mem[dirname_off + 4], 0);
    }

    /// Over-long dirName is truncated to 31 chars + NUL (matches
    /// RPCS3's `strcpy_trunc(dst[32], src)` semantics).
    #[test]
    fn auto_load_truncates_oversize_dir_name() {
        let (mut rt, unit) = fixture();
        let dir_name_addr = 0x1_0000u32;
        // 40 'A' bytes + NUL.
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
        // 31 'A' bytes followed by a NUL terminator at offset 31.
        assert_eq!(&mem[dirname_off..dirname_off + 31], &[b'A'; 31][..]);
        assert_eq!(mem[dirname_off + 31], 0);
    }

    /// `statGet.fileList` is populated from `setBuf.buf` so titles
    /// dereferencing fileList land on a valid title-supplied buffer
    /// even when fileListNum=0.
    #[test]
    fn auto_load_threads_set_buf_buf_into_file_list() {
        let (mut rt, unit) = fixture();
        let set_buf_addr = 0x2_0000u32;
        // Write `buf = 0xCAFEC0DE` into setBuf.OFF_BUF.
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

    /// AutoLoad2's eighth arg is `userdata`; the handler stages
    /// it into `cbResult.userdata` so the title's callback observes
    /// the value the title passed in.
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
        args[2] = 0; // dirName = NULL
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
        args[4] = 0; // setBuf = NULL
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
        args[5] = 0; // funcStat = NULL
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
        args[6] = 0; // funcFile = NULL
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args);
        assert_eq!(
            rt.registry_mut().drain_syscall_return(unit),
            Some(error::as_r3(error::PARAM)),
        );
    }

    #[test]
    fn auto_load_marks_handler_as_mutated() {
        // Stubs that call `set_return` flip `mutated = true` in the
        // adapter, so the NID must NOT land in
        // `handlers_without_mutation`. A future no-op stub that
        // forgets to set r3 would silently appear in that map; pin
        // the contract here.
        let (mut rt, unit) = fixture();
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args_with_pointers());
        let map = rt.hle_handlers_without_mutation();
        assert!(
            !map.contains_key(&save_nid::AUTO_LOAD),
            "AUTO_LOAD wrote r3 via set_return; must not appear in \
             handlers_without_mutation: {map:?}",
        );
    }

    /// PARAM rejection (NULL pointer in a mandatory slot) takes the
    /// fast set_return path: no heap allocation, no parking, no
    /// pending request. Pinned because the heap-and-park rewrite
    /// makes "happy path" the heavyweight branch; a regression that
    /// always allocates would trip here.
    #[test]
    fn auto_load_param_rejection_emits_no_heap_or_park() {
        let (mut rt, unit) = fixture();
        let mut args = args_with_pointers();
        args[2] = 0; // dirName = NULL
        let heap_before = rt.hle_heap_watermark();
        dispatch(&mut rt, unit, save_nid::AUTO_LOAD, &args);
        assert_eq!(rt.hle_heap_watermark(), heap_before);
        assert_eq!(
            rt.registry().effective_status(unit),
            Some(UnitStatus::Runnable),
        );
        assert!(rt.hle.pending_callback_spawn.is_none());
    }

    /// Two units call AutoLoad. unit_a's NULL-dirName rejection
    /// returns PARAM without parking; unit_b's happy path parks for
    /// funcStat. Each unit's state stays isolated; a shared-slot
    /// regression would clobber one with the other.
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

        // unit_b: happy path. Parks for funcStat; r3 unset until resume.
        dispatch(&mut rt, unit_b, save_nid::AUTO_LOAD, &args_with_pointers());
        assert_eq!(rt.registry_mut().drain_syscall_return(unit_b), None);
        assert!(rt.hle.pending_callback_spawn.is_some());
    }

    // ----- resume_after_stat coverage -----

    /// Drive a resume scenario: write `result` into the cbResult
    /// region at the OFF_RESULT slot, run resume_after_stat, return
    /// the parent's r3.
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
        // Until the funcFile loop is wired, OK_NEXT is reported as
        // FAILURE so titles see a real error rather than silent
        // success. This test pins the temporary mapping so adding
        // the loop forces an explicit update here.
        assert_eq!(
            drive_resume(cb_result::OK_NEXT),
            error::as_r3(error::FAILURE)
        );
    }

    #[test]
    fn resume_after_stat_ok_last_noconfirm_is_param() {
        // RPCS3's savedata_op (line 1630) rejects OK_LAST_NOCONFIRM
        // from funcStat; only funcFile/funcDone may use it.
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
        // Title-side bug: callback writes a value outside the
        // documented cb_result discriminant set. We map to PARAM
        // rather than panicking; the title gets a deterministic
        // error code instead of UB.
        assert_eq!(drive_resume(0x7FFF_FFFF), error::as_r3(error::PARAM));
        assert_eq!(drive_resume(-100), error::as_r3(error::PARAM));
    }

    #[test]
    fn resume_after_stat_unmapped_cb_result_maps_to_internal() {
        // The resume reads cbResult.result from guest memory; if
        // the title corrupted the pointer (or the runtime handed
        // back an unmapped address), the read fails and the parent
        // sees CELL_SAVEDATA_ERROR_INTERNAL rather than a silent
        // zero (which would look like CELL_OK).
        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let unit = rt
            .registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        // Address well past the mapped region.
        resume_after_stat(&mut rt, unit, 0xFF00_0000, 0, 0, 0, [0; 8]);
        assert_eq!(
            rt.registry_mut().drain_syscall_return(unit),
            Some(error::as_r3(error::INTERNAL)),
        );
    }

    #[test]
    fn every_cell_save_data_namespace_nid_routes_per_owned() {
        // For every NID in the cellSaveData typed namespace, assert
        // dispatch returns Some iff the NID is in OWNED_NIDS, and
        // None otherwise. Catches the drift case where a future
        // edit adds a `match` arm without updating OWNED_NIDS, or
        // vice versa, across the entire cellSaveData family.
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
        // Cross-check OWNED_NIDS against the ABI-side namespace
        // OWNED list. The abi list flows into
        // `cellgov_ps3_abi::nid::ALL_HLE_OWNED` and from there into
        // `cellgov_ppu::prx::HLE_IMPLEMENTED_NIDS`; pinning the
        // dispatcher's claimed set to the abi-side source-of-truth
        // catches the drift case where a NID is claimed by one and
        // not the other (which would either let a real PRX export
        // shadow the stub at load time, or make bound trampolines
        // fall through to the unclaimed-NID path).
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
