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
mod tests {
    use super::*;

    fn tty_args(fd: u64, buf: u64, len: u64) -> [u64; 9] {
        [403, fd, buf, len, 0, 0, 0, 0, 0]
    }

    #[test]
    fn classify_tty_capture_happy_path_returns_bytes_and_small_fd() {
        let mem = b"hello\0padding".to_vec();
        let args = tty_args(1, 0, 5);
        let decision = classify_tty_capture(&args, &mem);
        assert_eq!(
            decision,
            TtyCaptureDecision::InBounds {
                fd: 1,
                fd_was_bogus: false,
                bytes: b"hello".to_vec(),
            }
        );
    }

    #[test]
    fn classify_tty_capture_narrows_wide_fd_and_flags_bogus() {
        let mem = b"ok".to_vec();
        let args = tty_args(u64::from(u32::MAX) + 1, 0, 2);
        let decision = classify_tty_capture(&args, &mem);
        assert_eq!(
            decision,
            TtyCaptureDecision::InBounds {
                fd: u32::MAX,
                fd_was_bogus: true,
                bytes: b"ok".to_vec(),
            }
        );
    }

    #[test]
    fn classify_tty_capture_flags_oob_when_end_exceeds_mem() {
        let mem = b"tiny!".to_vec();
        let args = tty_args(1, 0, 10);
        let decision = classify_tty_capture(&args, &mem);
        assert_eq!(
            decision,
            TtyCaptureDecision::Oob {
                buf: 0,
                len: 10,
                mem_len: 5,
            }
        );
    }

    #[test]
    fn classify_tty_capture_flags_oob_on_checked_add_overflow() {
        let mem = vec![0u8; 16];
        let buf = usize::MAX as u64;
        let args = tty_args(1, buf, 8);
        let decision = classify_tty_capture(&args, &mem);
        assert!(
            matches!(decision, TtyCaptureDecision::Oob { .. }),
            "usize::MAX + 8 must classify as Oob, got {decision:?}"
        );
    }

    #[test]
    fn classify_tty_capture_keeps_full_buffer_above_4kib() {
        let mem = vec![b'x'; 8192];
        let args = tty_args(1, 0, 8000);
        let decision = classify_tty_capture(&args, &mem);
        match decision {
            TtyCaptureDecision::InBounds {
                fd,
                fd_was_bogus,
                bytes,
            } => {
                assert_eq!(fd, 1);
                assert!(!fd_was_bogus);
                assert_eq!(bytes.len(), 8000);
            }
            other => panic!("expected InBounds, got {other:?}"),
        }
    }

    #[test]
    fn classify_tty_capture_zero_len_with_garbage_buf_is_inbounds() {
        let mem = b"only-16-bytes!!!".to_vec();
        let args = tty_args(1, 0xDEAD_BEEF, 0);
        let decision = classify_tty_capture(&args, &mem);
        assert_eq!(
            decision,
            TtyCaptureDecision::InBounds {
                fd: 1,
                fd_was_bogus: false,
                bytes: Vec::new(),
            }
        );
    }

    #[test]
    fn classify_tty_capture_zero_len_at_mem_end_is_inbounds() {
        let mem = vec![0u8; 16];
        let args = tty_args(1, 16, 0);
        let decision = classify_tty_capture(&args, &mem);
        assert_eq!(
            decision,
            TtyCaptureDecision::InBounds {
                fd: 1,
                fd_was_bogus: false,
                bytes: Vec::new(),
            }
        );
    }
}
