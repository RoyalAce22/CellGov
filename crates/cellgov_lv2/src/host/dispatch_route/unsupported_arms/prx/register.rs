//! The `_sys_prx_*` module and library registration arms, and the manual import linking they run.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::guest_struct::{read_be_u32, read_be_u64, GuestStruct};
use crate::host::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    /// `_sys_prx_register_module` (484): binds a CoreOS caller's own
    /// import tables against the firmware export map.
    ///
    /// A caller that clears `type` bit 0 asks for no binding and gets
    /// CELL_OK. Any other process that asks for one gets
    /// CELL_PRX_ERROR_ELF_IS_REGISTERED.
    ///
    /// The struct layout is
    /// [`cellgov_ps3_abi::lv2::prx::register_module_option`]. `size`
    /// selects the form:
    ///
    /// - `0x1c` and `0x20` are the legacy forms: the arm takes
    ///   `type = 0` and reads no further field.
    /// - `0x30` carries the module type and the caller's stub table as
    ///   `(ea, size)`.
    ///
    /// [`Self::link_manual_imports`] binds a CoreOS caller's stub table
    /// against the resolved firmware exports, each entry under the
    /// library name it carries. A NID that library does not export
    /// stays unresolved and counts once in
    /// `prx_register_module_unresolved`; the counter does not record
    /// which library missed.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for a null `pOpt` or a `size` naming none of
    ///   the three modelled forms.
    /// - `CELL_EFAULT` when `pOpt` is unreadable or the struct would
    ///   not fit inside the 32-bit guest address space.
    /// - `CELL_EINVAL` when `pOpt` carries high bits, per
    ///   [`Lv2Host::narrow_u32_args`].
    pub(in crate::host::dispatch_route) fn dispatch_prx_register_module(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::prx::{
            register_module_option as opt_layout, CELL_PRX_ERROR_ELF_IS_REGISTERED,
        };
        use cellgov_ps3_abi::lv2::syscall;

        let Some([opt]) =
            self.narrow_u32_args(syscall::SYS_PRX_REGISTER_MODULE, [("pOpt", args[1])])
        else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        if opt == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        // The gate covers the whole read reach, so every field address
        // below is sound without a per-field check. The sc 481 / 482 /
        // 494 arms bound their option structs the same way.
        if opt.checked_add(opt_layout::TOUCHED_LEN).is_none() {
            self.log_invariant_break(
                "dispatch.prx_register_module_p_opt_wraps",
                format_args!(
                    "_sys_prx_register_module option struct [pOpt, pOpt+{touched:#x}) wraps \
                     u32: pOpt={opt:#010x}; returning CELL_EFAULT (struct does not fit in the \
                     32-bit guest address space)",
                    touched = opt_layout::TOUCHED_LEN
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let Some(size) = read_be_u64(rt, u64::from(opt)) else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        // A legacy form carries no type word; treating it as type = 0
        // skips the branch entirely, so it needs no field reads here.
        let (module_type, stub_ea, stub_size) = match size {
            s if opt_layout::LEGACY_SIZES.contains(&s) => (0u64, 0u32, 0u32),
            opt_layout::SIZE => {
                let Some(t) = read_be_u64(rt, u64::from(opt + opt_layout::TYPE_OFFSET)) else {
                    return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
                };
                let (Some(ea), Some(sz)) = (
                    read_be_u32(rt, u64::from(opt + opt_layout::STUB_EA_OFFSET)),
                    read_be_u32(rt, u64::from(opt + opt_layout::STUB_SIZE_OFFSET)),
                ) else {
                    return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
                };
                (t, ea, sz)
            }
            _ => {
                // The ABI module records this layout as unestablished.
                // A fourth form is the witness that would fix it, so
                // the break record keeps the size the caller declared.
                self.log_invariant_break(
                    "dispatch.prx_register_module_unknown_struct_size",
                    format_args!(
                        "_sys_prx_register_module pOpt={opt:#010x} declares size={size:#x}, \
                         none of the modelled forms; that layout's offsets are unestablished, \
                         so no field is read and CELL_EINVAL is returned"
                    ),
                );
                return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
            }
        };
        self.obs.prx_register_module_count += 1;

        if module_type & opt_layout::TYPE_MANUAL_IMPORTS == 0 {
            return Lv2Dispatch::immediate(0);
        }
        if !self.is_coreos() {
            // Only a CoreOS process may hand the kernel its own tables.
            return Lv2Dispatch::immediate(CELL_PRX_ERROR_ELF_IS_REGISTERED.into());
        }
        self.obs.prx_register_module_manual_count += 1;
        let effects = self.link_manual_imports(stub_ea, stub_size, requester, rt, tick);
        Lv2Dispatch::Immediate { code: 0, effects }
    }

    /// Bind the import table at `[stub_ea, stub_ea + stub_size)` against
    /// the firmware export map, returning one `SharedWriteIntent` per
    /// resolved GOT slot.
    ///
    /// The caller-supplied table is guest data and may be uninitialised
    /// -- a CoreOS module can reach this arm with a garbage pointer and
    /// a huge size. Every read is bounds-checked through the runtime and
    /// a failed read ends the walk rather than faulting the host; an
    /// unresolved NID is left alone so the guest's own stub address
    /// stays in the slot. Each way the walk can stop early names its
    /// own invariant break.
    fn link_manual_imports(
        &mut self,
        stub_ea: u32,
        stub_size: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Vec<Effect> {
        use cellgov_ps3_abi::format::elf::{
            PRX_IMPORT_ENTRY_MIN_SIZE, PRX_IMPORT_NAME_PTR_OFFSET, PRX_IMPORT_NIDS_PTR_OFFSET,
            PRX_IMPORT_NUM_FUNC_OFFSET, PRX_IMPORT_SIZE_OFFSET, PRX_IMPORT_STUB_PTR_OFFSET,
            PRX_NAME_MAX_LEN,
        };

        let mut effects = Vec::new();
        let Some(table_end) = stub_ea.checked_add(stub_size) else {
            self.log_invariant_break(
                "dispatch.prx_register_module_table_wraps",
                format_args!(
                    "import table [0x{stub_ea:08x}, +0x{stub_size:x}) wraps u32; \
                     nothing linked"
                ),
            );
            return effects;
        };
        let mut cursor = stub_ea;
        while cursor < table_end {
            let Some(hdr) =
                rt.read_committed(u64::from(cursor), PRX_IMPORT_ENTRY_MIN_SIZE as usize)
            else {
                self.log_invariant_break(
                    "dispatch.prx_register_module_entry_unreadable",
                    format_args!(
                        "import entry header at 0x{cursor:08x} is unreadable; the walk \
                         stops short of the table end 0x{table_end:08x}"
                    ),
                );
                break;
            };
            let entry_size = hdr[PRX_IMPORT_SIZE_OFFSET];
            if entry_size < PRX_IMPORT_ENTRY_MIN_SIZE {
                self.log_invariant_break(
                    "dispatch.prx_register_module_entry_size_invalid",
                    format_args!(
                        "import entry at 0x{cursor:08x} declares size {entry_size} below the \
                         {PRX_IMPORT_ENTRY_MIN_SIZE}-byte minimum; the walk stops short of \
                         the table end 0x{table_end:08x}"
                    ),
                );
                break;
            }
            let entry = GuestStruct::new(hdr);
            let func_count = entry.u16_at(PRX_IMPORT_NUM_FUNC_OFFSET);
            let nids_ptr = entry.u32_at(PRX_IMPORT_NIDS_PTR_OFFSET);
            let stub_ptr = entry.u32_at(PRX_IMPORT_STUB_PTR_OFFSET);
            let name_ptr = entry.u32_at(PRX_IMPORT_NAME_PTR_OFFSET);
            // The cap and the lossy decode mirror the loader-side
            // decoders that mint the `firmware_exports` keys
            // (`cellgov_ppu::prx::read_cstring`,
            // `cellgov_ppu::sprx::parse::read_cstring`): a name the
            // loader accepted must produce the identical lookup key
            // here, or the library silently never resolves.
            let library = match rt.read_committed_until(u64::from(name_ptr), PRX_NAME_MAX_LEN, 0) {
                Some(bytes) => self
                    .derived
                    .firmware_exports
                    .get(String::from_utf8_lossy(bytes).as_ref()),
                None => {
                    // An unreadable name resolves under no library:
                    // the entry's NIDs still walk (so the witness
                    // counts them) but every lookup misses.
                    self.log_invariant_break(
                        "dispatch.prx_register_module_name_unreadable",
                        format_args!(
                            "import entry at 0x{cursor:08x}: library-name pointer \
                             0x{name_ptr:08x} is unreadable or unterminated within \
                             {PRX_NAME_MAX_LEN} bytes; its {func_count} import NID(s) \
                             stay unresolved",
                        ),
                    );
                    None
                }
            };
            for i in 0..u32::from(func_count) {
                let (Some(nid_at), Some(slot_at)) = (
                    nids_ptr.checked_add(i * 4).map(u64::from),
                    stub_ptr.checked_add(i * 4).map(u64::from),
                ) else {
                    self.log_invariant_break(
                        "dispatch.prx_register_module_nid_table_truncated",
                        format_args!(
                            "import entry at 0x{cursor:08x}: NID slot {i} of {func_count} \
                             wraps u32 (nids=0x{nids_ptr:08x} stubs=0x{stub_ptr:08x}); the \
                             remaining NIDs stay unresolved"
                        ),
                    );
                    break;
                };
                let Some(nid) = read_be_u32(rt, nid_at) else {
                    self.log_invariant_break(
                        "dispatch.prx_register_module_nid_table_truncated",
                        format_args!(
                            "import entry at 0x{cursor:08x}: NID {i} of {func_count} at \
                             0x{nid_at:08x} is unreadable; the remaining NIDs stay unresolved"
                        ),
                    );
                    break;
                };
                let Some(&opd) = library.and_then(|lib| lib.get(&nid)) else {
                    self.obs.prx_register_module_unresolved += 1;
                    continue;
                };
                effects.push(Effect::shared_write(
                    ByteRange::contiguous_u32(slot_at as u32, 4),
                    WritePayload::from_slice(&opd.to_be_bytes()),
                    requester,
                    tick,
                ));
                self.obs.prx_register_module_linked += 1;
            }
            let Some(next) = cursor.checked_add(u32::from(entry_size)) else {
                self.log_invariant_break(
                    "dispatch.prx_register_module_entry_advance_wraps",
                    format_args!(
                        "import entry at 0x{cursor:08x} plus its size {entry_size} wraps u32; \
                         the walk stops short of the table end 0x{table_end:08x}"
                    ),
                );
                break;
            };
            cursor = next;
        }
        effects
    }

    /// `_sys_prx_register_library` (486): gates the library
    /// descriptor, then returns CELL_OK (the kernel's no-match
    /// success path).
    ///
    /// Associating the descriptor with a loaded module's export table
    /// is not modelled; CellGov publishes every firmware module's
    /// exports at boot, so a caller-registered library adds no
    /// resolvable symbol.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` when `library` is null or unmapped; the address
    ///   is checked before the descriptor is touched.
    /// - `CELL_EINVAL` when `library` carries high bits, per
    ///   [`Lv2Host::narrow_u32_args`].
    pub(in crate::host::dispatch_route) fn dispatch_prx_register_library(
        &mut self,
        args: [u64; 8],
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::syscall;

        let Some([library]) =
            self.narrow_u32_args(syscall::SYS_PRX_REGISTER_LIBRARY, [("library", args[0])])
        else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        if library == 0 || rt.read_committed(u64::from(library), 1).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        Lv2Dispatch::immediate(0)
    }
}
