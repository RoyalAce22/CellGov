//! What a stopped SPU thread asks of LV2, read from its `SPU_Status`
//! word.

use cellgov_ps3_abi::hw::spu::{
    SPU_STATUS_C, SPU_STATUS_H, SPU_STATUS_I, SPU_STATUS_P, SPU_STATUS_STOP_CODE_SHIFT,
    SPU_STOP_CODE_MASK,
};
use cellgov_ps3_abi::lv2::spu::stop_code;

/// The LV2 meaning of an SPU thread's stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuThreadStop {
    /// `sys_spu_thread_exit`: the thread ends with the status in its
    /// outbound mailbox.
    ThreadExit,
    /// `sys_spu_thread_group_exit`: every thread of the group ends with
    /// the status in the outbound mailbox.
    GroupExit,
    /// `spu_thread_group_yield`: the thread resumes.
    Yield,
    /// The stop is no request LV2 serves, and the thread group stops on
    /// an error.
    Error(SpuThreadError),
}

/// Why an SPU thread's stop is a thread-group error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SpuThreadError {
    /// A stop-and-signal whose code names no LV2 service CellGov
    /// serves.
    #[error("stop-and-signal code 0x{0:04x} names no LV2 service CellGov serves")]
    UnservedStopCode(u16),
    /// An exit whose outbound mailbox held no status.
    #[error("stop-and-signal code 0x{0:04x} with an empty SPU_WrOutMbox")]
    ExitWithoutStatus(u16),
    /// A halt instruction whose condition held.
    #[error("halt instruction")]
    Halt,
    /// A word that is not an SPU instruction.
    #[error("invalid instruction")]
    InvalidInstruction,
    /// A channel instruction the channel does not allow.
    #[error("invalid channel instruction")]
    InvalidChannel,
    /// A status word that records no stop cause.
    #[error("stop with no recorded cause (SPU_Status 0x{0:08x})")]
    NoCause(u32),
}

impl SpuThreadStop {
    /// The meaning of a stop whose `SPU_Status` word is `status`.
    ///
    /// [CBEA p:93 s:8.5.2] StopCode (bits 0:15) is valid only with P; C and I name an SPU error.
    /// [CBEA p:94 s:8.5.2] H names a halt and P a stop-and-signal.
    /// [CBEA p:263 s:21.4] an invalid instruction or channel instruction raises an SPU error, not a program stop.
    pub fn from_status(status: u32) -> Self {
        if status & SPU_STATUS_P != 0 {
            let code = ((status >> SPU_STATUS_STOP_CODE_SHIFT) & SPU_STOP_CODE_MASK) as u16;
            return match code {
                stop_code::THREAD_EXIT => Self::ThreadExit,
                stop_code::GROUP_EXIT => Self::GroupExit,
                stop_code::YIELD => Self::Yield,
                code => Self::Error(SpuThreadError::UnservedStopCode(code)),
            };
        }
        let error = if status & SPU_STATUS_I != 0 {
            SpuThreadError::InvalidInstruction
        } else if status & SPU_STATUS_C != 0 {
            SpuThreadError::InvalidChannel
        } else if status & SPU_STATUS_H != 0 {
            SpuThreadError::Halt
        } else {
            SpuThreadError::NoCause(status)
        };
        Self::Error(error)
    }
}

#[cfg(test)]
#[path = "tests/spu_stop_tests.rs"]
mod tests;
