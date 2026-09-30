//! MFC command parameters and the checks the MFC makes on them.
//!
//! The MFC latches a command's parameters when the SPU writes the
//! command, and checks them when it processes the command, asynchronous
//! to the instruction stream. A failed check suspends the queue and
//! raises one of two class 0 interrupts:
//!
//! - a DMA alignment error
//! - an invalid DMA command
//!
//! The checks are a pure function of the latched parameters, so
//! [`validate`] runs them once, and the queue acts on the verdict when it
//! processes the command.
//!
//! [CBEA p:57 s:7.2] an unaligned DMA suspends queue processing and raises a DMA alignment interrupt; Table 7-6 lists the command and alignment errors.
//! [CBEA p:113 s:9.1.1] the parameters' validity is checked asynchronous to the instruction stream.

use cellgov_ps3_abi::hw::spu::{
    MFC_ADDRESS_LOW_BITS, MFC_CLASS0_ALIGNMENT, MFC_CLASS0_INVALID_COMMAND,
    MFC_LIST_ADDRESS_LOW_BITS, MFC_SIZE_RESERVED_MASK, MFC_SNDSIG_SIZE, MFC_TAG_ID_RESERVED_MASK,
    MFC_TRANSFER_SIZE_MAX,
};

/// The parameter channels an MFC command latches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MfcParameters {
    /// `MFC_LSA`: the local-store address.
    pub lsa: u32,
    /// `MFC_EAH`: the effective address's high word.
    pub eah: u32,
    /// `MFC_EAL`: the effective address's low word, or a list's address.
    pub eal: u32,
    /// `MFC_Size`: the transfer size, or a list's size.
    pub size: u32,
    /// `MFC_TagID`: the tag group, as the guest wrote it.
    pub tag: u32,
}

impl MfcParameters {
    /// The 64-bit effective address `MFC_EAH || MFC_EAL`.
    pub const fn ea(self) -> u64 {
        (self.eah as u64) << 32 | self.eal as u64
    }
}

/// Which rows of Table 7-6 a command answers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MfcCommandClass {
    /// A put or get and their fenced and barrier forms.
    Transfer,
    /// A sndsig and its forms.
    SendSignal,
    /// A list put or get.
    List,
    /// getllar, putllc and putlluc.
    Atomic,
    /// mfcsync, mfceieio and barrier.
    Synchronization,
}

/// Which MFC interrupt a refused command raises.
///
/// [CBEA p:263 s:21.4 Table 21-3] the DMA alignment and invalid DMA command interrupts are class 0; the MFC data-segment and data-storage interrupts are class 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MfcExceptionClass {
    /// A DMA alignment error.
    Alignment,
    /// An invalid DMA command.
    InvalidCommand,
    /// An effective address outside every segment.
    DataSegment,
    /// An effective address with no mapping, or one the access may not use.
    DataStorage,
}

impl MfcExceptionClass {
    /// The class's bit in the class 0 interrupt status register.
    ///
    /// `None` for a class 1 exception, which has no bit in that register.
    pub const fn class0_status_bit(self) -> Option<u64> {
        match self {
            Self::Alignment => Some(MFC_CLASS0_ALIGNMENT),
            Self::InvalidCommand => Some(MFC_CLASS0_INVALID_COMMAND),
            Self::DataSegment | Self::DataStorage => None,
        }
    }
}

/// Why the MFC refuses a command.
///
/// - An opcode or parameter fails one row of Table 7-6.
/// - An effective address does not translate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum MfcCommandError {
    /// `MFC_TagID` sets a reserved bit, bits 0:26.
    #[error("tag 0x{0:08x} sets reserved bits")]
    ReservedTagBits(u32),
    /// `MFC_Size` sets a reserved bit, bits 0:16.
    #[error("size 0x{0:08x} sets reserved bits")]
    ReservedSizeBits(u32),
    /// A transfer or list size above 16 KB.
    #[error("size 0x{0:08x} is above 16 KB")]
    SizeTooLarge(u32),
    /// A transfer size other than 0, 1, 2, 4, 8 or a multiple of 16.
    #[error("transfer size 0x{0:08x} is not 1, 2, 4, 8 or a multiple of 16")]
    SizeUnaligned(u32),
    /// A sndsig size other than 4.
    #[error("sndsig size 0x{0:08x} is not 4")]
    SendSignalSize(u32),
    /// A local-store address not aligned for the transfer size, or a
    /// list's local-store address not doubleword aligned.
    #[error("local-store address 0x{lsa:08x} is not aligned for size 0x{size:08x}")]
    LocalStoreUnaligned {
        /// The local-store address.
        lsa: u32,
        /// The transfer size.
        size: u32,
    },
    /// The effective address's low four bits differ from the
    /// local-store address's.
    #[error("effective address 0x{ea:016x} and local-store address 0x{lsa:08x} differ in their low four bits")]
    AddressLowBitsDiffer {
        /// The local-store address.
        lsa: u32,
        /// The effective address.
        ea: u64,
    },
    /// A list address not doubleword aligned.
    #[error("list address 0x{0:08x} is not doubleword aligned")]
    ListAddressUnaligned(u32),
    /// An opcode the architecture neither defines nor reserves.
    #[error("opcode 0x{0:04x} is illegal")]
    IllegalOpcode(u32),
    /// An opcode in the reserved range.
    #[error("opcode 0x{0:04x} is reserved")]
    ReservedOpcode(u32),
    /// A defined command the SPU command queue does not accept: one with
    /// an `s` modifier.
    #[error("opcode 0x{0:04x} is a proxy-queue command")]
    ProxyOnlyCommand(u32),
    /// An effective address outside every segment.
    #[error("effective address 0x{ea:016x} is outside every segment")]
    DataSegment {
        /// The transfer's effective address.
        ea: u64,
    },
    /// An effective address with no mapping, or one the access may not use.
    #[error("effective address 0x{ea:016x} does not translate for the access")]
    DataStorage {
        /// The transfer's effective address.
        ea: u64,
    },
}

impl MfcCommandError {
    /// The interrupt the error raises.
    ///
    /// [CBEA p:118 s:9.1.6] a segment fault raises the MFC data-segment interrupt; a mapping fault or a protection violation raises the MFC data-storage interrupt.
    /// [CBEA p:57 s:7.2 Table 7-6] an invalid tag, an invalid opcode and a command the queue does not accept are DMA command errors; the size and address rows are DMA alignment errors.
    pub const fn class(self) -> MfcExceptionClass {
        match self {
            Self::ReservedTagBits(_)
            | Self::IllegalOpcode(_)
            | Self::ReservedOpcode(_)
            | Self::ProxyOnlyCommand(_) => MfcExceptionClass::InvalidCommand,
            Self::ReservedSizeBits(_)
            | Self::SizeTooLarge(_)
            | Self::SizeUnaligned(_)
            | Self::SendSignalSize(_)
            | Self::LocalStoreUnaligned { .. }
            | Self::AddressLowBitsDiffer { .. }
            | Self::ListAddressUnaligned(_) => MfcExceptionClass::Alignment,
            Self::DataSegment { .. } => MfcExceptionClass::DataSegment,
            Self::DataStorage { .. } => MfcExceptionClass::DataStorage,
        }
    }

    /// A stable code for the error, one per variant, for the queue's
    /// sync-state lanes.
    pub const fn code(self) -> u8 {
        match self {
            Self::ReservedTagBits(_) => 1,
            Self::ReservedSizeBits(_) => 2,
            Self::SizeTooLarge(_) => 3,
            Self::SizeUnaligned(_) => 4,
            Self::SendSignalSize(_) => 5,
            Self::LocalStoreUnaligned { .. } => 6,
            Self::AddressLowBitsDiffer { .. } => 7,
            Self::ListAddressUnaligned(_) => 8,
            Self::IllegalOpcode(_) => 9,
            Self::ReservedOpcode(_) => 10,
            Self::ProxyOnlyCommand(_) => 11,
            Self::DataSegment { .. } => 12,
            Self::DataStorage { .. } => 13,
        }
    }
}

/// A command the MFC refuses when it processes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InvalidMfcCommand {
    /// The command word the SPU wrote.
    pub word: u32,
    /// The latched parameters.
    pub params: MfcParameters,
    /// The first check the command fails.
    pub error: MfcCommandError,
}

/// Checks `params` against the Table 7-6 rows that apply to `class`.
///
/// The tag check runs first, because the MFC need not report an
/// alignment error beside a command error. Atomic and synchronization
/// commands take none of the alignment checks, and this function does
/// not check their tag.
///
/// [CBEA p:57 s:7.2 Table 7-6] footnote 1: the alignment checks do not apply to mfcsync, mfceieio, barrier and the atomic commands; footnote 2: an alignment error might not be reported if a command error is present.
/// [CBEA p:116 s:9.1.4] a transfer size is 0, 1, 2, 4, 8, 16 or a multiple of 16, up to 16 KB.
///
/// # Errors
///
/// The first [`MfcCommandError`] the parameters fail.
pub fn validate(class: MfcCommandClass, params: MfcParameters) -> Result<(), MfcCommandError> {
    if matches!(
        class,
        MfcCommandClass::Atomic | MfcCommandClass::Synchronization
    ) {
        return Ok(());
    }
    if params.tag & MFC_TAG_ID_RESERVED_MASK != 0 {
        return Err(MfcCommandError::ReservedTagBits(params.tag));
    }
    if params.size & MFC_SIZE_RESERVED_MASK != 0 {
        return Err(MfcCommandError::ReservedSizeBits(params.size));
    }
    if params.size > MFC_TRANSFER_SIZE_MAX {
        return Err(MfcCommandError::SizeTooLarge(params.size));
    }
    match class {
        MfcCommandClass::List => {
            // [CBEA p:57 s:7.2 Table 7-6] a list's local-store address is doubleword aligned, and so is its list address.
            if params.lsa & MFC_LIST_ADDRESS_LOW_BITS != 0 {
                return Err(MfcCommandError::LocalStoreUnaligned {
                    lsa: params.lsa,
                    size: params.size,
                });
            }
            if params.eal & MFC_LIST_ADDRESS_LOW_BITS != 0 {
                return Err(MfcCommandError::ListAddressUnaligned(params.eal));
            }
            Ok(())
        }
        MfcCommandClass::SendSignal if params.size != MFC_SNDSIG_SIZE => {
            Err(MfcCommandError::SendSignalSize(params.size))
        }
        MfcCommandClass::Transfer | MfcCommandClass::SendSignal => {
            let alignment = match params.size {
                0 | 1 => 1,
                2 | 4 | 8 => params.size,
                size if size.is_multiple_of(16) => 16,
                size => return Err(MfcCommandError::SizeUnaligned(size)),
            };
            if !params.lsa.is_multiple_of(alignment) {
                return Err(MfcCommandError::LocalStoreUnaligned {
                    lsa: params.lsa,
                    size: params.size,
                });
            }
            let low = u64::from(MFC_ADDRESS_LOW_BITS);
            if params.ea() & low != u64::from(params.lsa) & low {
                return Err(MfcCommandError::AddressLowBitsDiffer {
                    lsa: params.lsa,
                    ea: params.ea(),
                });
            }
            Ok(())
        }
        MfcCommandClass::Atomic | MfcCommandClass::Synchronization => Ok(()),
    }
}

#[cfg(test)]
#[path = "tests/command_tests.rs"]
mod tests;
