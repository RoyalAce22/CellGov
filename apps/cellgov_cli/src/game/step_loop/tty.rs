#[derive(Debug, PartialEq, Eq)]
pub(in crate::game) enum TtyCaptureDecision {
    /// Buffer fits in mapped memory, or `len == 0` (buf not dereferenced).
    InBounds {
        fd: u32,
        fd_was_bogus: bool,
        bytes: Vec<u8>,
    },
    Oob {
        buf: usize,
        len: usize,
        mem_len: usize,
    },
}

/// Bytes are captured at full fidelity; display layers bound output width.
pub(in crate::game) fn classify_tty_capture(
    args: &[u64; 9],
    mem_bytes: &[u8],
) -> TtyCaptureDecision {
    let buf = args[2] as usize;
    let len = args[3] as usize;
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
    let end = buf.checked_add(len);
    if end.is_none_or(|e| e > mem_bytes.len()) {
        return TtyCaptureDecision::Oob {
            buf,
            len,
            mem_len: mem_bytes.len(),
        };
    }
    let bytes = mem_bytes[buf..buf + len].to_vec();
    TtyCaptureDecision::InBounds {
        fd,
        fd_was_bogus,
        bytes,
    }
}

#[cfg(test)]
#[path = "tests/tty_tests.rs"]
mod tests;
