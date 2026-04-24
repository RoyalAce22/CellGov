//! PS3 LV2 `CellError` code database.
//!
//! Symbols, hex codes, and descriptions mirror
//! `tools/rpcs3-src/rpcs3/Emu/Cell/ErrorCodes.h:104-166`
//! byte-for-byte. `CELL_OK` lives in the header's
//! `CellNotAnError : s32` enum and is excluded from [`ENTRIES`].

/// A PS3 LV2 error code with its symbol and header description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv2Error {
    /// Numeric code.
    pub code: u32,
    /// Symbol name, e.g. `"CELL_EPERM"`.
    pub symbol: &'static str,
    /// Verbatim trailing comment from the RPCS3 header.
    pub description: &'static str,
}

/// Widens to the `u64` shape the dispatch pipeline uses for syscall
/// return codes; `errno::CELL_XXX.into()` keeps `errno::` in the path
/// so every errno-emitting site is greppable.
impl From<Lv2Error> for u64 {
    fn from(err: Lv2Error) -> u64 {
        err.code as u64
    }
}

/// Success sentinel. Not a `CellError` member, not in [`ENTRIES`].
pub const CELL_OK: Lv2Error = Lv2Error {
    code: 0,
    symbol: "CELL_OK",
    description: "",
};

// Binding `symbol` to `stringify!($name)` makes a typo between the
// ident and the string structurally impossible.
macro_rules! errno_table {
    ( $( $name:ident = $code:literal , $desc:literal ; )* ) => {
        $(
            #[doc = $desc]
            pub const $name: Lv2Error = Lv2Error {
                code: $code,
                symbol: stringify!($name),
                description: $desc,
            };
        )*

        /// Every `CellError` entry, ascending by code.
        pub const ENTRIES: &[&Lv2Error] = &[
            $( & $name , )*
        ];
    };
}

errno_table! {
    CELL_EAGAIN       = 0x8001_0001, "The resource is temporarily unavailable";
    CELL_EINVAL       = 0x8001_0002, "An invalid argument value is specified";
    CELL_ENOSYS       = 0x8001_0003, "The feature is not yet implemented";
    CELL_ENOMEM       = 0x8001_0004, "Memory allocation failure";
    CELL_ESRCH        = 0x8001_0005, "The resource with the specified identifier does not exist";
    CELL_ENOENT       = 0x8001_0006, "The file does not exist";
    CELL_ENOEXEC      = 0x8001_0007, "The file is in unrecognized format";
    CELL_EDEADLK      = 0x8001_0008, "Resource deadlock is avoided";
    CELL_EPERM        = 0x8001_0009, "The operation is not permitted";
    CELL_EBUSY        = 0x8001_000A, "The device or resource is busy";
    CELL_ETIMEDOUT    = 0x8001_000B, "The operation is timed out";
    CELL_EABORT       = 0x8001_000C, "The operation is aborted";
    CELL_EFAULT       = 0x8001_000D, "Invalid memory access";
    CELL_ENOCHILD     = 0x8001_000E, "Process has no child(s)";
    CELL_ESTAT        = 0x8001_000F, "State of the target thread is invalid";
    CELL_EALIGN       = 0x8001_0010, "Alignment is invalid.";
    CELL_EKRESOURCE   = 0x8001_0011, "Shortage of the kernel resources";
    CELL_EISDIR       = 0x8001_0012, "The file is a directory";
    CELL_ECANCELED    = 0x8001_0013, "Operation canceled";
    CELL_EEXIST       = 0x8001_0014, "Entry already exists";
    CELL_EISCONN      = 0x8001_0015, "Port is already connected";
    CELL_ENOTCONN     = 0x8001_0016, "Port is not connected";
    CELL_EAUTHFAIL    = 0x8001_0017, "Program authentication fail";
    CELL_ENOTMSELF    = 0x8001_0018, "The file is not a MSELF";
    CELL_ESYSVER      = 0x8001_0019, "System version error";
    CELL_EAUTHFATAL   = 0x8001_001A, "Fatal system error";
    CELL_EDOM         = 0x8001_001B, "Math domain violation";
    CELL_ERANGE       = 0x8001_001C, "Math range violation";
    CELL_EILSEQ       = 0x8001_001D, "Illegal multi-byte sequence in input";
    CELL_EFPOS        = 0x8001_001E, "File position error";
    CELL_EINTR        = 0x8001_001F, "Syscall was interrupted";
    CELL_EFBIG        = 0x8001_0020, "File too large";
    CELL_EMLINK       = 0x8001_0021, "Too many links";
    CELL_ENFILE       = 0x8001_0022, "File table overflow";
    CELL_ENOSPC       = 0x8001_0023, "No space left on device";
    CELL_ENOTTY       = 0x8001_0024, "Not a TTY";
    CELL_EPIPE        = 0x8001_0025, "Broken pipe";
    CELL_EROFS        = 0x8001_0026, "Read-only filesystem (write fail)";
    CELL_ESPIPE       = 0x8001_0027, "Illegal seek (e.g. seek on pipe)";
    CELL_E2BIG        = 0x8001_0028, "Arg list too long";
    CELL_EACCES       = 0x8001_0029, "Access violation";
    CELL_EBADF        = 0x8001_002A, "Invalid file descriptor";
    CELL_EIO          = 0x8001_002B, "Filesystem mounting failed (actually IO error...EIO)";
    CELL_EMFILE       = 0x8001_002C, "Too many files open";
    CELL_ENODEV       = 0x8001_002D, "No device";
    CELL_ENOTDIR      = 0x8001_002E, "Not a directory";
    CELL_ENXIO        = 0x8001_002F, "No such device or IO";
    CELL_EXDEV        = 0x8001_0030, "Cross-device link error";
    CELL_EBADMSG      = 0x8001_0031, "Bad Message";
    CELL_EINPROGRESS  = 0x8001_0032, "In progress";
    CELL_EMSGSIZE     = 0x8001_0033, "Message size error";
    CELL_ENAMETOOLONG = 0x8001_0034, "Name too long";
    CELL_ENOLCK       = 0x8001_0035, "No lock";
    CELL_ENOTEMPTY    = 0x8001_0036, "Not empty";
    CELL_ENOTSUP      = 0x8001_0037, "Not supported";
    CELL_EFSSPECIFIC  = 0x8001_0038, "File-system specific error";
    CELL_EOVERFLOW    = 0x8001_0039, "Overflow occured";
    CELL_ENOTMOUNTED  = 0x8001_003A, "Filesystem not mounted";
    CELL_ENOTSDATA    = 0x8001_003B, "Not SData";
    CELL_ESDKVER      = 0x8001_003C, "Incorrect version in sys_load_param";
    CELL_ENOLICDISC   = 0x8001_003D, "Pointer is null. Similar than 0x8001003E but with some PARAM.SFO parameter (TITLE_ID?) embedded.";
    CELL_ENOLICENT    = 0x8001_003E, "Pointer is null";
}

/// Look up an error entry by code; `None` for codes outside the
/// `CellError` block, including `CELL_OK`.
///
/// O(n) scan over [`ENTRIES`] (~60 entries, diagnostic paths only).
pub fn lookup(code: u32) -> Option<&'static Lv2Error> {
    ENTRIES.iter().copied().find(|e| e.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_code_is_unique() {
        let mut seen = BTreeSet::new();
        for entry in ENTRIES {
            assert!(
                seen.insert(entry.code),
                "duplicate errno code 0x{:08x} ({})",
                entry.code,
                entry.symbol,
            );
        }
    }

    #[test]
    fn every_symbol_has_cell_e_prefix() {
        for entry in ENTRIES {
            assert!(
                entry.symbol.starts_with("CELL_E"),
                "symbol {:?} (code 0x{:08x}) does not start \
                 with CELL_E",
                entry.symbol,
                entry.code,
            );
        }
    }

    #[test]
    fn lookup_hits_and_misses() {
        assert_eq!(lookup(0x8001_0009), Some(&CELL_EPERM));
        assert!(lookup(0xDEAD_BEEF).is_none());
        assert!(lookup(0).is_none());
    }
}
