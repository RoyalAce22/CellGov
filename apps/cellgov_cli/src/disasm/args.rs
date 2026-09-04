//! The one `dev disasm` input constraint the parser cannot express.

/// Why a `dev disasm` address was rejected.
// [PPC-Book1 p:7 s:1.7 Instruction formats] every instruction is a
// four-byte word on a four-byte boundary, so a misaligned start
// address decodes bytes that span two instructions.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum ArgError {
    #[error("--vaddr 0x{_0:016x} is not 4-byte aligned; PowerPC instructions are aligned words")]
    UnalignedVaddr(u64),
}

/// # Errors
///
/// [`ArgError::UnalignedVaddr`] for an address that is not a multiple
/// of four.
pub(super) fn check_alignment(vaddr: u64) -> Result<(), ArgError> {
    if vaddr.is_multiple_of(4) {
        Ok(())
    } else {
        Err(ArgError::UnalignedVaddr(vaddr))
    }
}

#[cfg(test)]
#[path = "tests/args_tests.rs"]
mod tests;
