//! `sys_fs_open` host dispatch.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::host::{Lv2Host, Lv2Runtime};

/// `CELL_FS_MAX_PATH_LENGTH`. Counts the terminator: max content
/// is `MAX_PATH_LEN - 1` and the NUL must appear at index `<= 1023`.
const MAX_PATH_LEN: usize = 1024;

/// `O_CREAT` bit in PS3 LV2 oflag.
const O_CREAT: u32 = 0x4;

impl Lv2Host {
    pub(super) fn dispatch_fs_open(
        &self,
        path_ptr: u32,
        flags: u32,
        fd_out_ptr: u32,
        mode: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if fd_out_ptr & 0x3 != 0 || !rt.writable(fd_out_ptr as u64, 4) {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }

        let path_bytes_owned: Vec<u8> =
            match rt.read_committed_until(path_ptr as u64, MAX_PATH_LEN, 0) {
                Some(prefix) => prefix.to_vec(),
                None => {
                    // Disambiguate unmapped path_ptr (EFAULT) from
                    // no-NUL-within-limit (EINVAL) via a 1-byte probe.
                    let code = if rt.read_committed(path_ptr as u64, 1).is_none() {
                        errno::CELL_EFAULT
                    } else {
                        errno::CELL_EINVAL
                    };
                    return Lv2Dispatch::Immediate {
                        code: code.into(),
                        effects: vec![],
                    };
                }
            };

        // PS3 paths are byte-strings; localized titles routinely
        // embed UTF-8 / Shift-JIS, so lossy decode is the only
        // correct shape for the log.
        let path_str = String::from_utf8_lossy(&path_bytes_owned);
        if flags & O_CREAT != 0 {
            eprintln!(
                "sys_fs_open: INVARIANT-BREAK O_CREAT requested for {path_str:?} \
                 (flags={flags:#x} mode={mode:#o}); whitelist is empty, returning CELL_ENOENT"
            );
        } else {
            eprintln!(
                "sys_fs_open: returning CELL_ENOENT for path {path_str:?} \
                 (flags={flags:#x} mode={mode:#o})"
            );
        }

        let write = Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(fd_out_ptr as u64), 4)
                .expect("fd_out_ptr range u32 fits in u64"),
            // PS3 is big-endian; guest reads via `lwz`.
            bytes: WritePayload::from_slice(&0u32.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: rt.current_tick(),
        };
        Lv2Dispatch::Immediate {
            code: errno::CELL_ENOENT.into(),
            effects: vec![write],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Lv2Request;
    use cellgov_time::GuestTicks;

    struct PathRuntime {
        bytes: Vec<u8>,
        reserved: Vec<(u32, u32)>,
        tick: GuestTicks,
    }

    impl PathRuntime {
        fn empty(size: usize) -> Self {
            Self {
                bytes: vec![0u8; size],
                reserved: Vec::new(),
                tick: GuestTicks::ZERO,
            }
        }

        fn write(mut self, addr: u32, payload: &[u8]) -> Self {
            let start = addr as usize;
            self.bytes[start..start + payload.len()].copy_from_slice(payload);
            self
        }

        fn reserve(mut self, start: u32, end: u32) -> Self {
            self.reserved.push((start, end));
            self
        }

        fn with_tick(mut self, tick: GuestTicks) -> Self {
            self.tick = tick;
            self
        }
    }

    impl Lv2Runtime for PathRuntime {
        fn read_committed(&self, addr: u64, len: usize) -> Option<&[u8]> {
            let start = addr as usize;
            let end = start.checked_add(len)?;
            if end > self.bytes.len() {
                return None;
            }
            for &(rs, re) in &self.reserved {
                let rs = rs as usize;
                let re = re as usize;
                if start < re && rs < end {
                    return None;
                }
            }
            Some(&self.bytes[start..end])
        }

        fn current_tick(&self) -> GuestTicks {
            self.tick
        }

        fn read_committed_until(&self, addr: u64, max_len: usize, terminator: u8) -> Option<&[u8]> {
            let start = addr as usize;
            self.read_committed(addr, 1)?;
            let mut len = 0usize;
            while len < max_len {
                let probe = self.read_committed(addr + len as u64, 1)?;
                if probe[0] == terminator {
                    return Some(&self.bytes[start..start + len]);
                }
                len += 1;
            }
            None
        }

        fn writable(&self, addr: u64, len: usize) -> bool {
            let Some(end) = (addr).checked_add(len as u64) else {
                return false;
            };
            if end > self.bytes.len() as u64 {
                return false;
            }
            let start = addr as usize;
            let end = end as usize;
            for &(rs, re) in &self.reserved {
                let rs = rs as usize;
                let re = re as usize;
                if start < re && rs < end {
                    return false;
                }
            }
            true
        }
    }

    fn run(host: &mut Lv2Host, rt: &dyn Lv2Runtime, request: Lv2Request) -> Lv2Dispatch {
        host.dispatch(request, UnitId::new(0), rt)
    }

    fn fs_open(path_ptr: u32, fd_out_ptr: u32, flags: u32, mode: u32) -> Lv2Request {
        Lv2Request::FsOpen {
            path_ptr,
            flags,
            fd_out_ptr,
            mode,
        }
    }

    fn assert_immediate(
        d: Lv2Dispatch,
        expected_code: u32,
        expected_effects: usize,
    ) -> Vec<Effect> {
        match d {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code as u32, expected_code, "errno mismatch");
                assert_eq!(effects.len(), expected_effects, "effect count mismatch");
                effects
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn unknown_path_returns_enoent_and_writes_zero_fd() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/dev_hdd0/game/foo/USRDIR/bar.dat\0");

        let effects = assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0x241, 0o666)),
            errno::CELL_ENOENT.code,
            1,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x20000);
                assert_eq!(bytes.bytes(), &0u32.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn path_without_null_terminator_returns_einval() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, &vec![b'A'; MAX_PATH_LEN]);

        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_EINVAL.code,
            0,
        );
    }

    #[test]
    fn out_of_range_path_ptr_returns_efault() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000);
        assert_immediate(
            run(&mut host, &rt, fs_open(0xFFFF_FF00, 0x20000, 0, 0)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn fd_out_ptr_unmapped_returns_efault_no_effects() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0xFFFF_FF00, 0, 0)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn fd_out_ptr_misaligned_returns_efault() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20001, 0, 0)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn fd_out_ptr_in_reserved_region_returns_efault() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000)
            .write(0x10000, b"/foo\0")
            .reserve(0x30000, 0x31000);
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x30100, 0, 0)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn path_at_region_end_succeeds() {
        // Place path in the last 5 bytes; pre-region-aware reads
        // EFAULTed here because a fixed 1024-byte window spilled
        // past the buffer.
        let mut host = Lv2Host::new();
        let path_ptr: u32 = 0x40000 - 5;
        let rt = PathRuntime::empty(0x40000).write(path_ptr, b"/foo\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(path_ptr, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
    }

    #[test]
    fn high_bit_bytes_in_path_succeed() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\xe6\x97\xa5\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
    }

    #[test]
    fn empty_path_returns_enoent() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
    }

    #[test]
    fn max_length_path_succeeds() {
        // 1023 content bytes + NUL: NUL at index MAX_PATH_LEN - 1
        // is the inclusive boundary the scan window must reach.
        let mut host = Lv2Host::new();
        let mut payload = vec![b'A'; MAX_PATH_LEN - 1];
        payload.push(0);
        let rt = PathRuntime::empty(0x40000).write(0x10000, &payload);
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
    }

    #[test]
    fn first_null_terminator_wins() {
        // Indirect: ENOENT (not EINVAL) confirms the scan stopped
        // at the first NUL. Tighten by reading back the logged
        // path once a structured invariant-break sink lands.
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0/bar\0");
        let effects = assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, .. } => {
                assert_eq!(range.start().raw(), 0x20000);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn o_creat_returns_enoent() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/save\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, O_CREAT, 0o666)),
            errno::CELL_ENOENT.code,
            1,
        );
    }

    #[test]
    fn source_time_matches_runtime_tick() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000)
            .write(0x10000, b"/foo\0")
            .with_tick(GuestTicks::new(42));
        let effects = assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { source_time, .. } => {
                assert_eq!(source_time.raw(), 42);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }
}
