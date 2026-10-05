//! PS3 LV2 `CellError` code database.
//!
//! Symbols and codes are the Cell OS Lv-2 error-code table: one
//! contiguous block starting at `0x8001_0001`, one symbol per code.
//! `CELL_OK` is the success sentinel, not a member of the block, and is
//! excluded from [`ENTRIES`].
//!
//! The comment on each row names the reference-console capture that
//! shows the code, or says why no capture holds it. A syscall leaves
//! the code in r3 sign-extended from 32 bits.

/// A PS3 LV2 error code with its symbol and description.
///
/// Value-catalogue entry, not a Rust-level error type: only `code`
/// flows to guest code via `impl From<Lv2ErrCode> for u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv2ErrCode {
    /// Numeric code.
    pub code: u32,
    /// Symbol name, e.g. `"CELL_EPERM"`.
    pub symbol: &'static str,
    /// One-line description of the condition the code names.
    ///
    /// Diagnostic text; nothing branches on it.
    pub description: &'static str,
}

/// The value a syscall leaves in r3: the code sign-extended from 32 bits,
/// as the `lv2_error_codes_sync` and `lv2_error_codes_fs` captures show.
impl From<Lv2ErrCode> for u64 {
    fn from(err: Lv2ErrCode) -> u64 {
        err.code as i32 as i64 as u64
    }
}

/// Success sentinel. Not a `CellError` member, not in [`ENTRIES`].
pub const CELL_OK: Lv2ErrCode = Lv2ErrCode {
    code: 0,
    symbol: "CELL_OK",
    description: "",
};

macro_rules! errno_table {
    ( $( $name:ident = $code:literal , $desc:literal ; )* ) => {
        $(
            #[doc = $desc]
            pub const $name: Lv2ErrCode = Lv2ErrCode {
                code: $code,
                symbol: stringify!($name),
                description: $desc,
            };
        )*

        /// Every `CellError` entry, ascending by code.
        pub const ENTRIES: &[&Lv2ErrCode] = &[
            $( & $name , )*
        ];
    };
}

errno_table! {
    // Not captured: it takes the process's whole 8192-entry object-id table to force.
    CELL_EAGAIN       = 0x8001_0001, "The resource is temporarily unavailable";
    // Console capture: lv2_error_codes_sync.
    CELL_EINVAL       = 0x8001_0002, "An invalid argument value is specified";
    // Console capture: lv2_syscall_bound.
    CELL_ENOSYS       = 0x8001_0003, "The feature is not yet implemented";
    // Console capture: lv2_error_codes_sync.
    CELL_ENOMEM       = 0x8001_0004, "Memory allocation failure";
    // Console capture: lv2_error_codes_sync.
    CELL_ESRCH        = 0x8001_0005, "The resource with the specified identifier does not exist";
    // Console capture: lv2_error_codes_fs.
    CELL_ENOENT       = 0x8001_0006, "The file does not exist";
    // Not captured: only an invalid SPU image gives it, and an invalid image can end in a fatal
    // system error.
    CELL_ENOEXEC      = 0x8001_0007, "The file is in unrecognized format";
    // Console capture: lv2_error_codes_sync.
    CELL_EDEADLK      = 0x8001_0008, "Resource deadlock is avoided";
    // Console capture: lv2_error_codes_sync.
    CELL_EPERM        = 0x8001_0009, "The operation is not permitted";
    // Console capture: lv2_error_codes_sync.
    CELL_EBUSY        = 0x8001_000A, "The device or resource is busy";
    // Console capture: lv2_error_codes_sync.
    CELL_ETIMEDOUT    = 0x8001_000B, "The operation is timed out";
    // Not captured: the one call documented to give it reads a console identifier.
    CELL_EABORT       = 0x8001_000C, "The operation is aborted";
    // Console capture: lv2_error_codes_sync.
    CELL_EFAULT       = 0x8001_000D, "Invalid memory access";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_ENOCHILD     = 0x8001_000E, "Process has no child(s)";
    // Not captured: the SPU thread-group and interrupt-thread state checks give it.
    CELL_ESTAT        = 0x8001_000F, "State of the target thread is invalid";
    // Console capture: lv2_error_codes_sync.
    CELL_EALIGN       = 0x8001_0010, "Alignment is invalid.";
    // Not captured: it takes 2^32 - 1 recursive locks of one mutex to force.
    CELL_EKRESOURCE   = 0x8001_0011, "Shortage of the kernel resources";
    // Console capture: lv2_error_codes_fs.
    CELL_EISDIR       = 0x8001_0012, "The file is a directory";
    // Console capture: lv2_destroy_parked.
    CELL_ECANCELED    = 0x8001_0013, "Operation canceled";
    // Console capture: lv2_error_codes_fs.
    CELL_EEXIST       = 0x8001_0014, "Entry already exists";
    // Console capture: lv2_event_port_answers.
    CELL_EISCONN      = 0x8001_0015, "Port is already connected";
    // Console capture: lv2_event_port_answers.
    CELL_ENOTCONN     = 0x8001_0016, "Port is not connected";
    // Not captured: only an image the SELF loader rejects gives it (see ENOEXEC).
    CELL_EAUTHFAIL    = 0x8001_0017, "Program authentication fail";
    // Not captured: an MSELF open of a file that is not one gives it.
    CELL_ENOTMSELF    = 0x8001_0018, "The file is not a MSELF";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_ESYSVER      = 0x8001_0019, "System version error";
    // Not captured: it names a fatal system error in the SELF loader.
    CELL_EAUTHFATAL   = 0x8001_001A, "Fatal system error";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_EDOM         = 0x8001_001B, "Math domain violation";
    // Not captured: only the file-system stream-read calls give it.
    CELL_ERANGE       = 0x8001_001C, "Math range violation";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_EILSEQ       = 0x8001_001D, "Illegal multi-byte sequence in input";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_EFPOS        = 0x8001_001E, "File position error";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_EINTR        = 0x8001_001F, "Syscall was interrupted";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_EFBIG        = 0x8001_0020, "File too large";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_EMLINK       = 0x8001_0021, "Too many links";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_ENFILE       = 0x8001_0022, "File table overflow";
    // Not captured: it takes a full drive to force.
    CELL_ENOSPC       = 0x8001_0023, "No space left on device";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_ENOTTY       = 0x8001_0024, "Not a TTY";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_EPIPE        = 0x8001_0025, "Broken pipe";
    // Not captured: the console gave it for a create under /dev_flash, in a run that ended in a
    // system-software error.
    CELL_EROFS        = 0x8001_0026, "Read-only filesystem (write fail)";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_ESPIPE       = 0x8001_0027, "Illegal seek (e.g. seek on pipe)";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_E2BIG        = 0x8001_0028, "Arg list too long";
    // Not captured: a write the file system refuses for permission gives it.
    CELL_EACCES       = 0x8001_0029, "Access violation";
    // Console capture: lv2_error_codes_fs.
    CELL_EBADF        = 0x8001_002A, "Invalid file descriptor";
    // Not captured: a device I/O failure gives it; no call can force one.
    CELL_EIO          = 0x8001_002B, "Input/output error";
    // Not captured: it takes more open files than the process limit to force.
    CELL_EMFILE       = 0x8001_002C, "Too many files open";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_ENODEV       = 0x8001_002D, "No device";
    // Console capture: lv2_error_codes_fs.
    CELL_ENOTDIR      = 0x8001_002E, "Not a directory";
    // Not captured: only the file-system stream and async calls give it.
    CELL_ENXIO        = 0x8001_002F, "No such device or IO";
    // Not captured: a rename across two file systems gives it.
    CELL_EXDEV        = 0x8001_0030, "Cross-device link error";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_EBADMSG      = 0x8001_0031, "Bad Message";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_EINPROGRESS  = 0x8001_0032, "In progress";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_EMSGSIZE     = 0x8001_0033, "Message size error";
    // Console capture: lv2_error_codes_fs.
    CELL_ENAMETOOLONG = 0x8001_0034, "Name too long";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_ENOLCK       = 0x8001_0035, "No lock";
    // Console capture: lv2_error_codes_fs.
    CELL_ENOTEMPTY    = 0x8001_0036, "Not empty";
    // Not captured: a call the file system does not support gives it.
    CELL_ENOTSUP      = 0x8001_0037, "Not supported";
    // Not captured: an internal file-system fault gives it; no call can force one.
    CELL_EFSSPECIFIC  = 0x8001_0038, "File-system specific error";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_EOVERFLOW    = 0x8001_0039, "Overflow occured";
    // Not captured: a path on a device that is not mounted gives it.
    CELL_ENOTMOUNTED  = 0x8001_003A, "Filesystem not mounted";
    // Not captured: an SDATA open of a file that is not one gives it.
    CELL_ENOTSDATA    = 0x8001_003B, "Not SData";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_ESDKVER      = 0x8001_003C, "Incorrect version in sys_load_param";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_ENOLICDISC   = 0x8001_003D, "Pointer is null. Similar than 0x8001003E but with some PARAM.SFO parameter (TITLE_ID?) embedded.";
    // No syscall or file-system call documents it; no probe can name a call that gives it.
    CELL_ENOLICENT    = 0x8001_003E, "Pointer is null";
}

/// Look up an error entry by code; `None` for codes outside the
/// `CellError` block, including `CELL_OK`.
///
/// O(n) scan over [`ENTRIES`] (~60 entries, diagnostic paths only).
pub fn lookup(code: u32) -> Option<&'static Lv2ErrCode> {
    ENTRIES.iter().copied().find(|e| e.code == code)
}

#[cfg(test)]
#[path = "tests/errno_tests.rs"]
mod tests;
