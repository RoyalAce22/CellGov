//! Generated SPU mailbox-write test ELF: header identity and embedded instruction sequences.

use super::*;

#[test]
fn gen_spu_mailbox_write_produces_valid_elf() {
    let elf = gen_spu_mailbox_write();
    assert_eq!(&elf[0..4], b"\x7fELF");
    assert_eq!(&elf[18..20], &[0x00, 0x15]);
    let entry = u64::from_be_bytes(elf[24..32].try_into().unwrap());
    assert_eq!(entry, 0x400000);
    let ilhu_bytes = spu::ilhu(2, 0x1337).to_be_bytes();
    assert!(elf.windows(4).any(|w| w == ilhu_bytes));
}

#[test]
fn gen_spu_mailbox_write_contains_syscall_instructions() {
    let elf = gen_spu_mailbox_write();
    let sc_bytes = [0x44, 0x00, 0x00, 0x02];
    let sc_count = elf.windows(4).filter(|w| *w == sc_bytes).count();
    assert_eq!(sc_count, 4);
}
