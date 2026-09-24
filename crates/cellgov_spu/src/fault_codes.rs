//! The guest fault codes the SPU unit yields in `FaultKind::Guest`.

use crate::exec::SpuFault;
use crate::state;
use cellgov_effects::FaultKind;

/// A fetch or an access outside local store.
///
/// The detail is one of:
///
/// - the program counter, on the fetch path;
/// - the raw address operand, on the load/store path;
/// - the staged `MFC_LSA`, where an MFC put, get, getllar or putllc
///   names a range local store cannot hold.
///
/// Local store spans 18 bits, so none of them fits the detail half and
/// the masked value is the address modulo 64 KB. [`LocalDiagnostics`]
/// carries the whole value beside the code: the fetch path's program
/// counter as `pc`, the other two as `faulting_ea`.
///
/// [`LocalDiagnostics`]: cellgov_exec::LocalDiagnostics
// [CBE-Handbook p:64 s:3.1.1 Local Store] Local store holds 256 KB, so an address inside it needs 18 bits.
pub(crate) const FAULT_LS_OUT_OF_RANGE: u32 = 0x0002_0000;
pub(crate) const FAULT_UNSUPPORTED_CHANNEL: u32 = 0x0003_0000;
/// An MFC command the model has no arm for.
///
/// The detail is the command word the guest wrote; its opcode sits in
/// the low byte, so the masked detail still names the command.
// [CBE-Handbook p:457 s:17.9.6 MFC Class ID and MFC Command Opcode Channel] The word written to this channel carries the transfer and replacement class ids in its high half and the MFC command opcode in its low byte.
pub(crate) const FAULT_UNSUPPORTED_MFC_CMD: u32 = 0x0004_0000;
pub(crate) const FAULT_DECODE_ERROR: u32 = 0x0005_0000;
/// A refused `rchcnt`, distinct from a refused `rdch` / `wrch` on the
/// same channel.
pub(crate) const FAULT_UNSUPPORTED_CHANNEL_COUNT: u32 = 0x0006_0000;
/// A parked MFC GET whose effective address resolves to no region. Its
/// low bits carry the transfer's tag id; [`LocalDiagnostics::faulting_ea`]
/// carries the effective address whole. A destination that escapes
/// local store is [`FAULT_LS_OUT_OF_RANGE`], as it is for a put.
///
/// [`LocalDiagnostics::faulting_ea`]: cellgov_exec::LocalDiagnostics::faulting_ea
pub(crate) const FAULT_MFC_GET_UNRESOLVED: u32 = 0x0007_0000;
/// An MFC command whose staged tag id is outside 0..31. The low bits
/// carry the value the guest wrote, masked to 16 bits.
pub(crate) const FAULT_MFC_TAG_ID_OUT_OF_RANGE: u32 = 0x0008_0000;
/// A synchronous MFC read -- `getllar` -- whose effective address
/// resolves to no region. The detail is the low 16 bits of the
/// effective address; [`LocalDiagnostics::faulting_ea`] carries it
/// whole.
///
/// [`LocalDiagnostics::faulting_ea`]: cellgov_exec::LocalDiagnostics::faulting_ea
pub(crate) const FAULT_MFC_READ_UNRESOLVED: u32 = 0x0009_0000;

/// The half of a fault code that carries the detail.
///
/// A class occupies the half above it. `cellgov_boot`'s fault report
/// splits a guest code at the same halfword boundary, so a detail that
/// reached a class bit would print as a different fault.
pub(crate) const FAULT_DETAIL_MASK: u32 = 0xFFFF;

/// Every class this crate raises, so the layout checks and the layout
/// tests cover one set.
const EVERY_FAULT_CLASS: [u32; 8] = [
    FAULT_LS_OUT_OF_RANGE,
    FAULT_UNSUPPORTED_CHANNEL,
    FAULT_UNSUPPORTED_MFC_CMD,
    FAULT_DECODE_ERROR,
    FAULT_UNSUPPORTED_CHANNEL_COUNT,
    FAULT_MFC_GET_UNRESOLVED,
    FAULT_MFC_TAG_ID_OUT_OF_RANGE,
    FAULT_MFC_READ_UNRESOLVED,
];

// The debug assertion in `guest_fault` compiles out under `--release`,
// so the layout is also checked here at compile time.
const _: () = {
    assert!(
        state::SPU_LS_SIZE as u32 > FAULT_DETAIL_MASK,
        "a local store inside the detail field would leave nothing to mask",
    );
    let mut i = 0;
    while i < EVERY_FAULT_CLASS.len() {
        assert!(
            EVERY_FAULT_CLASS[i] & FAULT_DETAIL_MASK == 0,
            "a fault class reaches into the detail field",
        );
        let mut j = i + 1;
        while j < EVERY_FAULT_CLASS.len() {
            assert!(
                EVERY_FAULT_CLASS[i] != EVERY_FAULT_CLASS[j],
                "two fault classes share a code",
            );
            j += 1;
        }
        i += 1;
    }
};

/// The class and detail each [`SpuFault`] reports.
pub(crate) fn guest_fault_for(fault: SpuFault) -> FaultKind {
    match fault {
        SpuFault::LsOutOfRange(a) => guest_fault(FAULT_LS_OUT_OF_RANGE, a),
        SpuFault::UnsupportedChannel { channel, .. } => {
            guest_fault(FAULT_UNSUPPORTED_CHANNEL, channel as u32)
        }
        SpuFault::UnsupportedMfcCommand(c) => guest_fault(FAULT_UNSUPPORTED_MFC_CMD, c),
        SpuFault::UnsupportedChannelCount(channel) => {
            guest_fault(FAULT_UNSUPPORTED_CHANNEL_COUNT, channel as u32)
        }
        SpuFault::TagIdOutOfRange(tag) => guest_fault(FAULT_MFC_TAG_ID_OUT_OF_RANGE, tag),
    }
}

/// One guest fault: `class` in the high half of the code, `detail`
/// masked into the low half.
///
/// The mask keeps a guest-chosen detail out of the class half; see
/// [`FAULT_DETAIL_MASK`]. The assertion covers a class added after
/// [`EVERY_FAULT_CLASS`].
pub(crate) fn guest_fault(class: u32, detail: u32) -> FaultKind {
    debug_assert!(
        class & FAULT_DETAIL_MASK == 0,
        "fault class 0x{class:08x} reaches into the detail field",
    );
    FaultKind::Guest(class | (detail & FAULT_DETAIL_MASK))
}

#[cfg(test)]
#[path = "tests/fault_code_tests.rs"]
mod fault_code_tests;
