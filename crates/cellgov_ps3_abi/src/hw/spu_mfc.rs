//! The defined MFC commands, the queues that accept each one, and the
//! class of an opcode.

/// Which MFC command queues accept a defined command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MfcQueues {
    /// The proxy queue and the SPU queue.
    Both,
    /// The proxy queue only: the commands with an `s` modifier.
    ProxyOnly,
    /// The SPU queue only: the list and atomic commands.
    SpuOnly,
}

/// One defined MFC command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MfcCommandDef {
    /// The command's mnemonic.
    pub mnemonic: &'static str,
    /// The 16-bit opcode.
    pub opcode: u16,
    /// The queues that accept it.
    pub queues: MfcQueues,
}

const fn def(mnemonic: &'static str, opcode: u16, queues: MfcQueues) -> MfcCommandDef {
    MfcCommandDef {
        mnemonic,
        opcode,
        queues,
    }
}

/// Every defined MFC command, in the order of the appendix tables.
///
/// [CBEA p:306 s:Appendix D Table D-2] the put and get commands and the queues each is supported on.
/// [CBEA p:307 s:Appendix D Table D-3] the SL1 storage control commands.
/// [CBEA p:308 s:Appendix D Table D-4] the synchronization commands; [CBEA p:308 s:Appendix D Table D-5] the atomic commands.
/// [CBEA p:51 s:7] an `s` command is proxy-queue only; a list or atomic command is SPU-queue only.
pub const MFC_COMMANDS: [MfcCommandDef; 39] = {
    use MfcQueues::{Both, ProxyOnly, SpuOnly};
    [
        def("put", 0x0020, Both),
        def("puts", 0x0028, ProxyOnly),
        def("putr", 0x0030, Both),
        def("putf", 0x0022, Both),
        def("putb", 0x0021, Both),
        def("putfs", 0x002A, ProxyOnly),
        def("putbs", 0x0029, ProxyOnly),
        def("putrf", 0x0032, Both),
        def("putrb", 0x0031, Both),
        def("putl", 0x0024, SpuOnly),
        def("putrl", 0x0034, SpuOnly),
        def("putlf", 0x0026, SpuOnly),
        def("putlb", 0x0025, SpuOnly),
        def("putrlf", 0x0036, SpuOnly),
        def("putrlb", 0x0035, SpuOnly),
        def("get", 0x0040, Both),
        def("gets", 0x0048, ProxyOnly),
        def("getf", 0x0042, Both),
        def("getb", 0x0041, Both),
        def("getfs", 0x004A, ProxyOnly),
        def("getbs", 0x0049, ProxyOnly),
        def("getl", 0x0044, SpuOnly),
        def("getlf", 0x0046, SpuOnly),
        def("getlb", 0x0045, SpuOnly),
        def("sdcrt", 0x0080, Both),
        def("sdcrtst", 0x0081, Both),
        def("sdcrz", 0x0089, Both),
        def("sdcrst", 0x008D, Both),
        def("sdcrf", 0x008F, Both),
        def("sndsig", 0x00A0, Both),
        def("sndsigf", 0x00A2, Both),
        def("sndsigb", 0x00A1, Both),
        def("barrier", 0x00C0, Both),
        def("mfceieio", 0x00C8, Both),
        def("mfcsync", 0x00CC, Both),
        def("getllar", 0x00D0, SpuOnly),
        def("putllc", 0x00B4, SpuOnly),
        def("putlluc", 0x00B0, SpuOnly),
        def("putqlluc", 0x00B8, SpuOnly),
    ]
};

/// The lowest opcode of the reserved range, which ends at x'FFFF'.
///
/// [CBEA p:57 s:7.1.3] reserved commands have opcodes x'8000' to x'FFFF'.
pub const MFC_RESERVED_OPCODE_BASE: u16 = 0x8000;

/// The class of an MFC command opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MfcOpcodeClass {
    /// A command the architecture defines.
    Defined(MfcCommandDef),
    /// An opcode reserved for implementation-dependent use.
    Reserved,
    /// Any other opcode.
    Illegal,
}

/// The class of `opcode`.
///
/// [CBEA p:53 s:7.1] a command that is neither defined nor reserved is illegal.
pub const fn mfc_opcode_class(opcode: u16) -> MfcOpcodeClass {
    if opcode >= MFC_RESERVED_OPCODE_BASE {
        return MfcOpcodeClass::Reserved;
    }
    let mut i = 0;
    while i < MFC_COMMANDS.len() {
        if MFC_COMMANDS[i].opcode == opcode {
            return MfcOpcodeClass::Defined(MFC_COMMANDS[i]);
        }
        i += 1;
    }
    MfcOpcodeClass::Illegal
}

#[cfg(test)]
#[path = "tests/spu_mfc_tests.rs"]
mod tests;
