//! The defined MFC commands and the class of an opcode.

use super::*;

#[test]
fn every_defined_opcode_is_distinct_and_classifies_as_itself() {
    for (i, def) in MFC_COMMANDS.iter().enumerate() {
        assert!(
            MFC_COMMANDS[..i].iter().all(|d| d.opcode != def.opcode),
            "{} repeats an opcode",
            def.mnemonic
        );
        assert_eq!(mfc_opcode_class(def.opcode), MfcOpcodeClass::Defined(*def));
    }
}

/// [CBEA p:51 s:7] only the `s` commands are proxy-queue only, and the list and atomic commands are SPU-queue only.
#[test]
fn the_s_commands_alone_are_proxy_only() {
    let proxy_only: Vec<&str> = MFC_COMMANDS
        .iter()
        .filter(|d| d.queues == MfcQueues::ProxyOnly)
        .map(|d| d.mnemonic)
        .collect();
    assert_eq!(
        proxy_only,
        ["puts", "putfs", "putbs", "gets", "getfs", "getbs"]
    );
    let spu_only: Vec<&str> = MFC_COMMANDS
        .iter()
        .filter(|d| d.queues == MfcQueues::SpuOnly)
        .map(|d| d.mnemonic)
        .collect();
    assert_eq!(
        spu_only,
        [
            "putl", "putrl", "putlf", "putlb", "putrlf", "putrlb", "getl", "getlf", "getlb",
            "getllar", "putllc", "putlluc", "putqlluc"
        ]
    );
}

/// [CBEA p:57 s:7.1.3] x'8000' to x'FFFF' is the reserved range; [CBEA p:53 s:7.1] anything else not defined is illegal.
#[test]
fn an_opcode_outside_the_tables_is_reserved_from_x8000_and_illegal_below() {
    assert_eq!(mfc_opcode_class(0x8000), MfcOpcodeClass::Reserved);
    assert_eq!(mfc_opcode_class(0x8020), MfcOpcodeClass::Reserved);
    assert_eq!(mfc_opcode_class(0xFFFF), MfcOpcodeClass::Reserved);
    assert_eq!(mfc_opcode_class(0x7FFF), MfcOpcodeClass::Illegal);
    assert_eq!(mfc_opcode_class(0x0000), MfcOpcodeClass::Illegal);
    assert_eq!(mfc_opcode_class(0x0027), MfcOpcodeClass::Illegal);
    assert_eq!(
        mfc_opcode_class(0x0120),
        MfcOpcodeClass::Illegal,
        "the high byte is part of the opcode, so a put's low byte is not enough"
    );
}

#[test]
fn a_command_word_classifies_its_whole_low_halfword() {
    let class = |raw| crate::hw::spu::MfcCmd::new(raw).class();
    assert!(matches!(class(0x0302_0020), MfcOpcodeClass::Defined(d) if d.mnemonic == "put"));
    assert_eq!(class(0x0000_0120), MfcOpcodeClass::Illegal);
    assert_eq!(class(0x0000_8020), MfcOpcodeClass::Reserved);
}
