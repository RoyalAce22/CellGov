//! Typed LV2 syscall requests.
//!
//! The PPU's `run_until_yield` packages syscall arguments into one of
//! these variants and yields with `YieldReason::Syscall`. The runtime
//! passes the request to `Lv2Host::dispatch`. The PPU crate does not
//! depend on this crate -- the runtime decodes the raw GPR values into
//! an `Lv2Request` at the boundary.

/// A typed LV2 syscall request.
///
/// Each variant carries the guest-address arguments the PPU placed in
/// GPRs 3..=10 before executing `sc`. All pointer fields are guest
/// effective addresses (u32 on PS3 despite the 64-bit ELF container).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2Request {
    /// sys_spu_image_open (156).
    SpuImageOpen {
        /// Guest address to populate with the `sys_spu_image_t` struct.
        img_ptr: u32,
        /// Guest address of the NUL-terminated path string.
        path_ptr: u32,
    },
    /// sys_spu_thread_group_create (170).
    SpuThreadGroupCreate {
        /// Guest address to write the allocated group id into.
        id_ptr: u32,
        /// Number of SPU threads in the group.
        num_threads: u32,
        /// Priority (not used by CellGov).
        priority: u32,
        /// Guest address of the attribute struct (opaque, not inspected).
        attr_ptr: u32,
    },
    /// sys_spu_thread_initialize (172).
    /// ABI: r3=thread_ptr, r4=group, r5=spu_num, r6=img_ptr, r7=attr_ptr, r8=arg_ptr
    SpuThreadInitialize {
        /// Guest address to write the allocated thread id into.
        thread_ptr: u32,
        /// Thread group id returned by a previous create call.
        group_id: u32,
        /// Slot index within the group (0-based).
        thread_num: u32,
        /// Guest address of the sys_spu_image_t struct (contains handle).
        img_ptr: u32,
        /// Guest address of the attribute struct (opaque).
        attr_ptr: u32,
        /// Guest address of `sys_spu_thread_argument`.
        arg_ptr: u32,
    },
    /// sys_spu_thread_group_start (173).
    SpuThreadGroupStart {
        /// Thread group id.
        group_id: u32,
    },
    /// sys_spu_thread_group_join (177).
    SpuThreadGroupJoin {
        /// Thread group id.
        group_id: u32,
        /// Guest address to write the exit cause into.
        cause_ptr: u32,
        /// Guest address to write the exit status into.
        status_ptr: u32,
    },
    /// sys_tty_write (403).
    TtyWrite {
        /// File descriptor (typically 0 for stdout).
        fd: u32,
        /// Guest address of the buffer to write.
        buf_ptr: u32,
        /// Number of bytes to write.
        len: u32,
        /// Guest address to store the number of bytes written.
        nwritten_ptr: u32,
    },
    /// sys_spu_thread_write_spu_mb (190).
    SpuThreadWriteMb {
        /// Thread id returned by sysSpuThreadInitialize.
        thread_id: u32,
        /// Value to deposit into the SPU's inbound mailbox.
        value: u32,
    },
    /// sys_process_exit (22).
    ProcessExit {
        /// Exit code.
        code: u32,
    },
    /// A syscall number that does not map to any known request.
    Unsupported {
        /// The raw syscall number from GPR 11.
        number: u64,
    },
}

/// Build an `Lv2Request` from the raw syscall number and GPR values.
///
/// The PPU places the syscall number in r11 and up to 8 arguments in
/// r3..=r10. This function maps the number to the typed request,
/// extracting the relevant arguments. Unknown syscalls produce
/// `Lv2Request::Unsupported`.
pub fn classify(syscall_num: u64, args: &[u64; 8]) -> Lv2Request {
    match syscall_num {
        156 => Lv2Request::SpuImageOpen {
            img_ptr: args[0] as u32,
            path_ptr: args[1] as u32,
        },
        170 => Lv2Request::SpuThreadGroupCreate {
            id_ptr: args[0] as u32,
            num_threads: args[1] as u32,
            priority: args[2] as u32,
            attr_ptr: args[3] as u32,
        },
        172 => Lv2Request::SpuThreadInitialize {
            thread_ptr: args[0] as u32,
            group_id: args[1] as u32,
            thread_num: args[2] as u32,
            img_ptr: args[3] as u32,
            attr_ptr: args[4] as u32,
            arg_ptr: args[5] as u32,
        },
        173 => Lv2Request::SpuThreadGroupStart {
            group_id: args[0] as u32,
        },
        177 | 178 => Lv2Request::SpuThreadGroupJoin {
            group_id: args[0] as u32,
            cause_ptr: args[1] as u32,
            status_ptr: args[2] as u32,
        },
        190 => Lv2Request::SpuThreadWriteMb {
            thread_id: args[0] as u32,
            value: args[1] as u32,
        },
        403 => Lv2Request::TtyWrite {
            fd: args[0] as u32,
            buf_ptr: args[1] as u32,
            len: args[2] as u32,
            nwritten_ptr: args[3] as u32,
        },
        22 => Lv2Request::ProcessExit {
            code: args[0] as u32,
        },
        n => Lv2Request::Unsupported { number: n },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_spu_image_open() {
        let args = [0x1000, 0x2000, 0, 0, 0, 0, 0, 0];
        let req = classify(156, &args);
        assert_eq!(
            req,
            Lv2Request::SpuImageOpen {
                img_ptr: 0x1000,
                path_ptr: 0x2000,
            }
        );
    }

    #[test]
    fn classify_thread_group_create() {
        let args = [0x3000, 2, 100, 0x4000, 0, 0, 0, 0];
        let req = classify(170, &args);
        assert_eq!(
            req,
            Lv2Request::SpuThreadGroupCreate {
                id_ptr: 0x3000,
                num_threads: 2,
                priority: 100,
                attr_ptr: 0x4000,
            }
        );
    }

    #[test]
    fn classify_thread_initialize() {
        let args = [0x6000, 1, 0, 0x7000, 0x8000, 0x9000, 0, 0];
        let req = classify(172, &args);
        assert_eq!(
            req,
            Lv2Request::SpuThreadInitialize {
                thread_ptr: 0x6000,
                group_id: 1,
                thread_num: 0,
                img_ptr: 0x7000,
                attr_ptr: 0x8000,
                arg_ptr: 0x9000,
            }
        );
    }

    #[test]
    fn classify_thread_group_start() {
        let args = [7, 0, 0, 0, 0, 0, 0, 0];
        let req = classify(173, &args);
        assert_eq!(req, Lv2Request::SpuThreadGroupStart { group_id: 7 });
    }

    #[test]
    fn classify_thread_group_join() {
        let args = [3, 0x6000, 0x7000, 0, 0, 0, 0, 0];
        let req = classify(177, &args);
        assert_eq!(
            req,
            Lv2Request::SpuThreadGroupJoin {
                group_id: 3,
                cause_ptr: 0x6000,
                status_ptr: 0x7000,
            }
        );
    }

    #[test]
    fn classify_tty_write() {
        let args = [0, 0x8000, 64, 0x9000, 0, 0, 0, 0];
        let req = classify(403, &args);
        assert_eq!(
            req,
            Lv2Request::TtyWrite {
                fd: 0,
                buf_ptr: 0x8000,
                len: 64,
                nwritten_ptr: 0x9000,
            }
        );
    }

    #[test]
    fn classify_process_exit() {
        let args = [0, 0, 0, 0, 0, 0, 0, 0];
        let req = classify(22, &args);
        assert_eq!(req, Lv2Request::ProcessExit { code: 0 });
    }

    #[test]
    fn classify_unknown_syscall() {
        let args = [0; 8];
        let req = classify(999, &args);
        assert_eq!(req, Lv2Request::Unsupported { number: 999 });
    }

    #[test]
    fn spu_thread_group_range_stubs_classify_as_unsupported() {
        let args = [0; 8];
        for n in [171, 174, 175, 176, 179, 180, 192] {
            let req = classify(n, &args);
            assert!(
                matches!(req, Lv2Request::Unsupported { .. }),
                "syscall {n} should be Unsupported"
            );
        }
    }
}
