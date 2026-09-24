//! The `_sys_prx_get_module_list` arm and the big-endian write it stages.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::guest_struct::{read_be_u32, read_be_u64};
use crate::host::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    /// Stage a big-endian `u64` write to guest memory.
    pub(super) fn write_be_u64(
        &self,
        requester: UnitId,
        addr: u32,
        value: u64,
        tick: GuestTicks,
    ) -> Effect {
        Effect::shared_write(
            ByteRange::contiguous_u32(addr, 8),
            WritePayload::from_slice(&value.to_be_bytes()),
            requester,
            tick,
        )
    }

    /// `_sys_prx_get_module_list` (494): fills `pInfo->idlist` and
    /// writes `pInfo->count`, filtering liblv2.sprx.
    ///
    /// The struct layout is
    /// [`cellgov_ps3_abi::lv2::prx::get_module_list_option`]. A caller
    /// that clears the fill-list flag gets CELL_OK. The arm reads no
    /// field of `pInfo` on that path, so a null pointer gets CELL_OK
    /// too.
    ///
    /// `size` selects the layout. A value other than the modelled one
    /// names a struct whose `max` / `count` / `idlist` sit elsewhere,
    /// and that layout is not modelled: the call fills nothing,
    /// answers CELL_OK, and logs a break.
    ///
    /// The fill stops at `pInfo->max` slots, or at the first slot that
    /// leaves the 32-bit guest address space; that slot names its own
    /// invariant break. `count` reports the slots written. A null
    /// `idlist` skips the slot writes and still writes `count`. The
    /// walk follows the registry's `BTreeMap` key order, so the bytes
    /// written do not depend on registration order.
    ///
    /// # Cross-module contract
    ///
    /// Slot writes and the trailing count write are co-emitted in one
    /// `Lv2Dispatch::Immediate` batch so `apply_lv2_effects` can
    /// commit them all-or-none.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` when `pInfo` carries high bits, per
    ///   [`Lv2Host::narrow_u32_args`]. The check runs before the
    ///   fill-list test, so it also answers a caller that asks for no
    ///   fill.
    /// - `CELL_EFAULT` for a null `pInfo`, for a struct that would not
    ///   fit inside the 32-bit guest address space, and for an
    ///   unreadable `size` / `max` / `idlist` field.
    pub(in crate::host::dispatch_route) fn dispatch_prx_get_module_list(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::prx::get_module_list_option as opt;
        use cellgov_ps3_abi::lv2::syscall;
        let modelled_size = opt::SIZE;

        let flags = args[0];
        let Some([p_info]) =
            self.narrow_u32_args(syscall::SYS_PRX_GET_MODULE_LIST, [("pInfo", args[1])])
        else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        if flags & opt::FLAG_FILL_LIST == 0 {
            return Lv2Dispatch::immediate(0);
        }
        if p_info == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        if p_info.checked_add(opt::TOUCHED_LEN).is_none() {
            self.log_invariant_break(
                "dispatch.prx_module_list_p_info_wraps",
                format_args!(
                    "sys_prx_get_module_list pInfo struct [p_info, p_info+{touched:#x}) wraps \
                     u32: pInfo={p_info:#010x}; returning CELL_EFAULT (struct does not fit in \
                     32-bit guest address space)",
                    touched = opt::TOUCHED_LEN
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let Some(declared_size) = read_be_u64(rt, u64::from(p_info)) else {
            self.log_invariant_break(
                "dispatch.prx_module_list_unreadable_pinfo",
                format_args!(
                    "sys_prx_get_module_list pInfo={p_info:#010x} size field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        if declared_size != modelled_size {
            self.log_invariant_break(
                "dispatch.prx_module_list_unknown_struct_size",
                format_args!(
                    "sys_prx_get_module_list pInfo={p_info:#010x} declares \
                     size={declared_size:#x}, not {modelled_size:#x}; the other \
                     layout is not modelled, so nothing is filled in and CELL_OK is \
                     returned"
                ),
            );
            return Lv2Dispatch::immediate(0);
        }
        let mut effects = Vec::new();
        let max_addr = p_info.wrapping_add(opt::MAX_OFFSET);
        let count_addr = p_info.wrapping_add(opt::COUNT_OFFSET);
        let idlist_ptr_addr = p_info.wrapping_add(opt::IDLIST_OFFSET);
        let Some(max) = read_be_u32(rt, u64::from(max_addr)) else {
            self.log_invariant_break(
                "dispatch.prx_module_list_unreadable_pinfo",
                format_args!(
                    "sys_prx_get_module_list pInfo={p_info:#010x} max field at \
                     {max_addr:#010x} unreadable; returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        let Some(idlist_ptr) = read_be_u32(rt, u64::from(idlist_ptr_addr)) else {
            self.log_invariant_break(
                "dispatch.prx_module_list_unreadable_pinfo",
                format_args!(
                    "sys_prx_get_module_list pInfo={p_info:#010x} idlist field at \
                     {idlist_ptr_addr:#010x} unreadable; returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        let liblv2_id = self
            .state
            .prx_registry
            .lookup_by_path("liblv2.sprx")
            .map(|e| e.kernel_id());
        let mut count: u32 = 0;
        // `idlist` is a guest pointer read out of the option struct, so
        // an array near the top of the address space cannot hold every
        // slot.
        let mut wrapped_slot: Option<u32> = None;
        if idlist_ptr != 0 {
            for kid in self.state.prx_registry.ids() {
                if Some(kid) == liblv2_id {
                    continue;
                }
                if count >= max {
                    break;
                }
                let slot = count
                    .checked_mul(opt::ID_SIZE)
                    .and_then(|off| idlist_ptr.checked_add(off))
                    .filter(|s| s.checked_add(opt::ID_SIZE).is_some());
                let Some(slot) = slot else {
                    wrapped_slot = Some(count);
                    break;
                };
                effects.push(Effect::shared_write(
                    ByteRange::contiguous_u32(slot, opt::ID_SIZE),
                    WritePayload::from_slice(&kid.to_be_bytes()),
                    requester,
                    tick,
                ));
                count += 1;
            }
        }
        if let Some(index) = wrapped_slot {
            self.log_invariant_break(
                "dispatch.prx_module_list_idlist_slot_wraps",
                format_args!(
                    "sys_prx_get_module_list id slot {index} at idlist_ptr+{index}*{id_size} \
                     leaves the 32-bit guest address space: idlist_ptr={idlist_ptr:#010x}; the \
                     fill stops there and count reports the {index} slot(s) written",
                    id_size = opt::ID_SIZE
                ),
            );
        }
        effects.push(Effect::shared_write(
            ByteRange::contiguous_u32(count_addr, opt::COUNT_SIZE),
            WritePayload::from_slice(&count.to_be_bytes()),
            requester,
            tick,
        ));
        Lv2Dispatch::Immediate { code: 0, effects }
    }
}
