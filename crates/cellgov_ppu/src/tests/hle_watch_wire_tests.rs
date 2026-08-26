//! `hle_watch::wire`: every record kind's builder produces its
//! declared length behind its kind byte, fields in declared order.

use super::wire::*;

fn gpr() -> [u64; 32] {
    let mut g = [0u64; 32];
    for (i, r) in g.iter_mut().enumerate() {
        *r = 0x1000 + i as u64;
    }
    g
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn le64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

#[test]
fn an_entry_record_carries_r3_to_r10_after_the_fixed_fields() {
    let rec = entry(
        7,
        0xAABB_CCDD,
        0x0010_0000,
        0x0010_0000,
        0x0020_0004,
        &gpr(),
    );
    assert_eq!(rec.len(), ENTRY_LEN);
    assert_eq!(rec[0], KIND_ENTRY);
    assert_eq!(le64(&rec, 1), 7);
    assert_eq!(le32(&rec, 9), 0xAABB_CCDD);
    assert_eq!(le32(&rec, 13), 0x0010_0000);
    assert_eq!(le32(&rec, 17), 0x0010_0000);
    assert_eq!(le32(&rec, 21), 0x0020_0004);
    for (i, arg) in (3..=10).enumerate() {
        assert_eq!(le64(&rec, 25 + 8 * i), 0x1000 + arg, "r{arg}");
    }
}

#[test]
fn an_exit_record_pairs_to_its_entry_and_carries_r3() {
    let rec = exit(9, 0x1234_5678, 7, 0x0020_0004, 0xCAFE);
    assert_eq!(rec.len(), EXIT_LEN);
    assert_eq!(rec[0], KIND_EXIT);
    assert_eq!(le64(&rec, 1), 9);
    assert_eq!(le32(&rec, 9), 0x1234_5678);
    assert_eq!(le64(&rec, 13), 7);
    assert_eq!(le32(&rec, 21), 0x0020_0004);
    assert_eq!(le64(&rec, 25), 0xCAFE);
}

#[test]
fn body_records_have_their_declared_lengths_and_kinds() {
    let g = gpr();
    let sc = body_syscall(1, 2, 3, 0x81, 0x100, &g);
    assert_eq!((sc.len(), sc[0]), (BODY_SYSCALL_LEN, KIND_BODY_SYSCALL));
    assert_eq!(le32(&sc, 21), 0x81, "syscall number precedes pc");
    assert_eq!(le32(&sc, 25), 0x100);
    assert_eq!(le64(&sc, 29), 0x1003, "first arg is r3");

    let ret = body_syscall_return(4, 2, 3, 0x81, 0x104, 0xFFFF_FFFF_8001_0002);
    assert_eq!(
        (ret.len(), ret[0]),
        (BODY_SYSCALL_RETURN_LEN, KIND_BODY_SYSCALL_RETURN)
    );
    assert_eq!(le64(&ret, 29), 0xFFFF_FFFF_8001_0002);

    let call = body_call(5, 2, 3, 0x108, 0x0030_0000, &g);
    assert_eq!((call.len(), call[0]), (BODY_CALL_LEN, KIND_BODY_CALL));
    assert_eq!(le32(&call, 21), 0x108);
    assert_eq!(le32(&call, 25), 0x0030_0000, "target follows pc");
    assert_eq!(le64(&call, 29), 0x1003, "first arg is r3");
}

#[test]
fn a_resolution_record_caps_the_name_at_255_bytes() {
    let short = resolution(0xA, 0xB, "sys_ppu_thread_create");
    assert_eq!(short[0], KIND_RESOLUTION);
    assert_eq!(
        short.len(),
        RESOLUTION_HEAD_LEN + "sys_ppu_thread_create".len()
    );
    assert_eq!(
        short[RESOLUTION_HEAD_LEN - 1],
        "sys_ppu_thread_create".len() as u8
    );
    assert_eq!(&short[RESOLUTION_HEAD_LEN..], b"sys_ppu_thread_create");

    let long_name = "x".repeat(300);
    let long = resolution(0xA, 0xB, &long_name);
    assert_eq!(long.len(), RESOLUTION_HEAD_LEN + 255);
    assert_eq!(long[RESOLUTION_HEAD_LEN - 1], 255);
}
