//! `sys_rsx_context_iomap` (672) dispatch.

use cellgov_ps3_abi::cell_errors as errno;
use cellgov_ps3_abi::process_address_space::{PS3_RSX_BASE, PS3_RSX_IOMAP_SIZE};
use cellgov_ps3_abi::sys_rsx::iomap;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

impl Lv2Host {
    /// Records the io -> ea mapping on the live `SysRsxContext`.
    ///
    /// # Cross-module contract
    ///
    /// The kernel-backed iomap region of [`PS3_RSX_IOMAP_SIZE`]
    /// bytes is composed by the boot pipeline; the title's later
    /// writes through the mapping land in writable guest memory.
    ///
    /// Only the most recent mapping is retained -- a second call
    /// overwrites the first. RPCS3 keys per-1 MiB-page in
    /// `iomap_table`; CellGov's single triple covers every
    /// modeled title (each issues one window through
    /// `_cellGcmInitBody`).
    ///
    /// # Errors
    ///
    /// Per `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_rsx.cpp:398-453`,
    /// `CELL_EINVAL` for: `context_id != 0x5555_5555`, `size == 0`,
    /// any of `io`/`ea`/`size` not 1 MiB-aligned, `ea + size`
    /// crossing into [`PS3_RSX_BASE`] (RPCS3's `local_mem_base`),
    /// or `io + size` exceeding the baked iomap region. Only the
    /// io-over-cap path logs `dispatch.sys_rsx_context_iomap_oversized`;
    /// the ea-range rejection is a plain EINVAL, matching RPCS3's
    /// undifferentiated gate.
    pub(in crate::host) fn dispatch_sys_rsx_context_iomap(
        &mut self,
        context_id: u32,
        io: u32,
        ea: u32,
        size: u32,
        _flags: u64,
    ) -> Lv2Dispatch {
        if context_id != iomap::CONTEXT_ID {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if size == 0
            || (io & iomap::ALIGN_MASK) != 0
            || (ea & iomap::ALIGN_MASK) != 0
            || (size & iomap::ALIGN_MASK) != 0
        {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        // u64 catches u32 wrap; PS3_RSX_BASE is RPCS3's local_mem_base.
        if u64::from(ea) + u64::from(size) > PS3_RSX_BASE {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        // u64 catches u32 wrap (e.g. io=0xFFF0_0000+size=0x10_0000
        // wraps to 0).
        const BAKED_IOMAP_SIZE: u64 = PS3_RSX_IOMAP_SIZE as u64;
        if u64::from(io) + u64::from(size) > BAKED_IOMAP_SIZE {
            self.log_invariant_break(
                "dispatch.sys_rsx_context_iomap_oversized",
                format_args!(
                    "sys_rsx_context_iomap io={io:#x}+size={size:#x} exceeds baked \
                     region {BAKED_IOMAP_SIZE:#x}; returning CELL_EINVAL"
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        self.rsx_context.iomap_io = io;
        self.rsx_context.iomap_ea = ea;
        self.rsx_context.iomap_size = size;
        Lv2Dispatch::immediate(errno::CELL_OK.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::rsx::test_helpers::context_allocate_request;
    use crate::host::test_support::FakeRuntime;
    use crate::request::Lv2Request;
    use cellgov_event::UnitId;

    fn allocate_context(host: &mut Lv2Host) {
        let rt = FakeRuntime::new(0x1_0000);
        let _ = host.dispatch(
            context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 0xA001),
            UnitId::new(0),
            &rt,
        );
    }

    fn iomap(host: &mut Lv2Host, context_id: u32, io: u32, ea: u32, size: u32) -> Lv2Dispatch {
        let rt = FakeRuntime::new(0x1_0000);
        host.dispatch(
            Lv2Request::SysRsxContextIomap {
                context_id,
                io,
                ea,
                size,
                flags: 0,
            },
            UnitId::new(0),
            &rt,
        )
    }

    #[test]
    fn valid_call_records_mapping_and_returns_ok() {
        let mut host = Lv2Host::new();
        allocate_context(&mut host);
        let d = iomap(&mut host, iomap::CONTEXT_ID, 0, 0x0010_0000, 0x0010_0000);
        let Lv2Dispatch::Immediate { code, effects } = d else {
            panic!("expected Immediate, got {d:?}");
        };
        assert_eq!(code, u64::from(errno::CELL_OK));
        assert!(effects.is_empty(), "iomap is purely state-recording");
        let ctx = host.sys_rsx_context();
        assert_eq!(ctx.iomap_io, 0);
        assert_eq!(ctx.iomap_ea, 0x0010_0000);
        assert_eq!(ctx.iomap_size, 0x0010_0000);
    }

    #[test]
    fn nonzero_io_records_offset() {
        let mut host = Lv2Host::new();
        allocate_context(&mut host);
        let d = iomap(
            &mut host,
            iomap::CONTEXT_ID,
            0x0010_0000,
            0x0020_0000,
            0x0010_0000,
        );
        assert_eq!(d, Lv2Dispatch::immediate(errno::CELL_OK.into()));
        assert_eq!(host.sys_rsx_context().iomap_io, 0x0010_0000);
    }

    #[test]
    fn wrong_context_id_returns_einval() {
        let mut host = Lv2Host::new();
        allocate_context(&mut host);
        let d = iomap(&mut host, 0xDEAD_BEEF, 0, 0x0010_0000, 0x0010_0000);
        assert_eq!(d, Lv2Dispatch::immediate(errno::CELL_EINVAL.into()));
    }

    #[test]
    fn iomap_before_allocate_keys_only_on_context_id() {
        // RPCS3 sys_rsx.cpp:398-453 does not gate on host-side
        // context-allocate state; CONTEXT_ID is the only handshake.
        let mut host = Lv2Host::new();
        let d = iomap(&mut host, iomap::CONTEXT_ID, 0, 0x0010_0000, 0x0010_0000);
        assert_eq!(d, Lv2Dispatch::immediate(errno::CELL_OK.into()));
    }

    #[test]
    fn zero_size_returns_einval() {
        let mut host = Lv2Host::new();
        allocate_context(&mut host);
        let d = iomap(&mut host, iomap::CONTEXT_ID, 0, 0x0010_0000, 0);
        assert_eq!(d, Lv2Dispatch::immediate(errno::CELL_EINVAL.into()));
    }

    #[test]
    fn misaligned_io_ea_or_size_returns_einval() {
        let mut host = Lv2Host::new();
        allocate_context(&mut host);
        for (io, ea, size, label) in [
            (1, 0x0010_0000, 0x0010_0000, "io"),
            (0, 0x0010_0001, 0x0010_0000, "ea"),
            (0, 0x0010_0000, 0x0000_1000, "size"),
        ] {
            let d = iomap(&mut host, iomap::CONTEXT_ID, io, ea, size);
            assert_eq!(
                d,
                Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
                "misaligned {label} must reject",
            );
        }
    }

    #[test]
    fn io_plus_size_overflow_returns_einval_and_logs() {
        // This io+size wraps to 0 in u32; any u32 comparison would
        // silently accept it.
        let mut host = Lv2Host::new();
        allocate_context(&mut host);
        let before = host.invariant_break_count();
        let d = iomap(&mut host, iomap::CONTEXT_ID, 0xFFF0_0000, 0, 0x0010_0000);
        assert_eq!(d, Lv2Dispatch::immediate(errno::CELL_EINVAL.into()));
        assert_eq!(host.invariant_break_count() - before, 1);
    }

    #[test]
    fn io_plus_size_at_exact_cap_is_ok() {
        let mut host = Lv2Host::new();
        allocate_context(&mut host);
        let cap = u32::try_from(PS3_RSX_IOMAP_SIZE).unwrap();
        let d = iomap(
            &mut host,
            iomap::CONTEXT_ID,
            cap - 0x0010_0000,
            0,
            0x0010_0000,
        );
        assert_eq!(d, Lv2Dispatch::immediate(errno::CELL_OK.into()));
    }

    #[test]
    fn oversized_size_returns_einval_and_logs_invariant_break() {
        let mut host = Lv2Host::new();
        allocate_context(&mut host);
        let breaks_before = host.invariant_break_count();
        let too_big = u32::try_from(PS3_RSX_IOMAP_SIZE).unwrap() + 0x0010_0000;
        let d = iomap(&mut host, iomap::CONTEXT_ID, 0, 0x0010_0000, too_big);
        assert_eq!(d, Lv2Dispatch::immediate(errno::CELL_EINVAL.into()));
        assert_eq!(host.invariant_break_count() - breaks_before, 1);
    }

    #[test]
    fn ea_plus_size_exceeds_local_mem_returns_einval() {
        // Asymmetry: ea-range rejection is plain EINVAL with no
        // invariant break, matching RPCS3's undifferentiated gate.
        let mut host = Lv2Host::new();
        allocate_context(&mut host);
        let before = host.invariant_break_count();
        let local_mem_base = u32::try_from(PS3_RSX_BASE).unwrap();
        let d = iomap(&mut host, iomap::CONTEXT_ID, 0, local_mem_base, 0x0010_0000);
        assert_eq!(d, Lv2Dispatch::immediate(errno::CELL_EINVAL.into()));
        assert_eq!(host.invariant_break_count(), before);
    }

    #[test]
    fn second_iomap_overwrites_first() {
        let mut host = Lv2Host::new();
        allocate_context(&mut host);
        let _ = iomap(&mut host, iomap::CONTEXT_ID, 0, 0x0010_0000, 0x0010_0000);
        let _ = iomap(
            &mut host,
            iomap::CONTEXT_ID,
            0x0010_0000,
            0x0020_0000,
            0x0010_0000,
        );
        let ctx = host.sys_rsx_context();
        assert_eq!((ctx.iomap_io, ctx.iomap_ea), (0x0010_0000, 0x0020_0000));
    }

    // WipEout's first iomap call asks for 0x0550_0000 bytes.
    const _: () = assert!(PS3_RSX_IOMAP_SIZE >= 0x0550_0000);
}
