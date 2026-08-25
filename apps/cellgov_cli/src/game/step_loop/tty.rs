use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, MemError};

#[derive(Debug, PartialEq, Eq)]
pub(in crate::game) enum TtyCaptureDecision {
    /// Buffer resolves in the caller's mapped memory, or `len == 0`
    /// (buf not dereferenced).
    InBounds {
        fd: u32,
        fd_was_bogus: bool,
        bytes: Vec<u8>,
    },
    /// Buffer does not resolve; `reason` names the region gap or the
    /// reserved region it landed in.
    Oob {
        buf: u64,
        len: u64,
        reason: MemError,
    },
}

/// Resolve a `sys_tty_write` buffer through `mem`, the caller's address
/// space, so a buffer on a stack or child-space region reads the same
/// way a guest load of it would.
///
/// Bytes are captured at full fidelity; display layers bound output width.
pub(in crate::game) fn classify_tty_capture(
    args: &[u64; 9],
    mem: &GuestMemory,
) -> TtyCaptureDecision {
    let buf = args[2];
    let len = args[3];
    // Narrow oversized fd to a sentinel rather than aliasing to a low fd.
    let (fd, fd_was_bogus) = match u32::try_from(args[1]) {
        Ok(fd) => (fd, false),
        Err(_) => (u32::MAX, true),
    };
    if len == 0 {
        return TtyCaptureDecision::InBounds {
            fd,
            fd_was_bogus,
            bytes: Vec::new(),
        };
    }
    // A range that wraps the address space is unmapped by definition;
    // its start is still the address worth naming.
    let resolved = ByteRange::new(GuestAddr::new(buf), len)
        .ok_or_else(|| MemError::Unmapped(mem.fault_context(buf)))
        .and_then(|range| mem.read_checked(range));
    match resolved {
        Ok(bytes) => TtyCaptureDecision::InBounds {
            fd,
            fd_was_bogus,
            bytes: bytes.to_vec(),
        },
        Err(reason) => TtyCaptureDecision::Oob { buf, len, reason },
    }
}

#[cfg(test)]
#[path = "tests/tty_tests.rs"]
mod tests;
