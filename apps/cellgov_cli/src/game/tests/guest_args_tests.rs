use super::{build_args_block, GuestArgsError};

const TOP: u64 = 0xD00F_FFF0;
const SIZE: u64 = 0x0010_0000;

fn be64(bytes: &[u8], off: usize) -> u64 {
    u64::from_be_bytes(bytes[off..off + 8].try_into().unwrap())
}

#[test]
fn single_arg_layout_matches_the_rpcs3_shape() {
    let block = build_args_block(TOP, SIZE, &["abc".to_string()]).unwrap();
    // 1 argv ptr + argv NULL + envp NULL -> 3 slots, padded to 4;
    // "abc\0" pads to 16.
    assert_eq!(block.bytes.len(), 4 * 8 + 16);
    assert_eq!(block.base, TOP - 48);
    assert_eq!(block.base % 16, 0);
    assert_eq!(block.initial_r1, block.base - 0x70);
    assert_eq!(block.initial_r1 % 16, 0);
    assert_eq!(block.argc, 1);
    assert_eq!(block.argv_addr, block.base);
    assert_eq!(block.envp_addr, block.base + 16);
    assert_eq!(be64(&block.bytes, 0), block.base + 32);
    assert_eq!(be64(&block.bytes, 8), 0);
    assert_eq!(be64(&block.bytes, 16), 0);
    assert_eq!(be64(&block.bytes, 24), 0);
    assert_eq!(&block.bytes[32..36], b"abc\0");
    assert!(block.bytes[36..].iter().all(|&b| b == 0));
}

#[test]
fn two_args_use_all_four_slots_and_stack_16_byte_string_chunks() {
    let args = ["/dev_flash/vsh/module/vsh.self", "--mode=gametool"];
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let block = build_args_block(TOP, SIZE, &args).unwrap();
    // 2 argv ptrs + argv NULL + envp NULL -> 4 slots exactly;
    // 31 bytes -> 32, 16 bytes -> 16.
    assert_eq!(block.bytes.len(), 4 * 8 + 32 + 16);
    assert_eq!(block.argc, 2);
    assert_eq!(block.envp_addr, block.base + 24);
    let s0 = be64(&block.bytes, 0);
    let s1 = be64(&block.bytes, 8);
    assert_eq!(s0, block.base + 32);
    assert_eq!(s1, s0 + 32);
    assert_eq!(be64(&block.bytes, 16), 0);
    assert_eq!(be64(&block.bytes, 24), 0);
    let o1 = (s1 - block.base) as usize;
    assert_eq!(&block.bytes[o1..o1 + 16], b"--mode=gametool\0");
}

#[test]
fn an_empty_string_arg_still_gets_a_pointer_and_a_nul_terminated_chunk() {
    // argc counts the empty entry and argv[0] must point at a real
    // "\0", not alias NULL: a guest strlen(argv[0]) walk has to see
    // a terminator inside the block.
    let block = build_args_block(TOP, SIZE, &[String::new()]).unwrap();
    assert_eq!(block.argc, 1);
    assert_eq!(block.bytes.len(), 4 * 8 + 16);
    let s0 = be64(&block.bytes, 0);
    assert_eq!(s0, block.base + 32);
    assert_eq!(block.bytes[32], 0);
}

#[test]
fn nul_byte_in_an_arg_is_rejected_with_its_index() {
    let args = vec!["ok".to_string(), "bad\0arg".to_string()];
    assert_eq!(
        build_args_block(TOP, SIZE, &args),
        Err(GuestArgsError::ArgContainsNul { index: 1 })
    );
}

#[test]
fn block_exceeding_the_stack_is_rejected() {
    let args = vec!["x".repeat(SIZE as usize)];
    assert!(matches!(
        build_args_block(TOP, SIZE, &args),
        Err(GuestArgsError::BlockTooLarge { .. })
    ));
}

#[test]
fn r1_sits_a_full_linkage_frame_below_the_pointer_table() {
    // 8(r1) and 16(r1) are the CR and LR save slots the entry's
    // prologue may store through; both must fall in the reserve, not
    // in the table.
    let block = build_args_block(TOP, SIZE, &["a".to_string(), "b".to_string()]).unwrap();
    assert!(block.initial_r1 + 16 < block.argv_addr);
    assert_eq!(block.argv_addr - block.initial_r1, 0x70);
}

#[test]
fn the_fit_check_counts_the_entry_frame_reserve() {
    // One arg: 4 table slots (0x20). A string chunk of 0xFFF70
    // makes total + 0x70 == SIZE exactly -> rejected; one 16-byte
    // granule less fits.
    let rejected = vec!["x".repeat(0xFFF6F)];
    assert!(matches!(
        build_args_block(TOP, SIZE, &rejected),
        Err(GuestArgsError::BlockTooLarge {
            total: 0xFFF90,
            stack_size: SIZE,
        })
    ));
    let fits = vec!["x".repeat(0xFFF5F)];
    let block = build_args_block(TOP, SIZE, &fits).unwrap();
    assert_eq!(block.initial_r1, TOP - 0xFFF80 - 0x70);
}
