//! The system manager's reply of service id 0x0A, held to the console
//! capture of the kernel's system manager object.

use super::system_manager_reply_console_tests::{
    capture_rows, word, KERNEL_TOC, MANAGER, MANAGER_SLOT,
};
use super::*;

/// The reply's payload and stored bytes at the first GameOS boot after
/// lv1 starts, from the captured word at reply bytes 4 to 7.
///
/// The setup fills the block at vaddr 0xC000D35B only when bit 0 of that
/// word is set [OFW-4.93 lv1 a:0x453E54-0x453E64]. With the bit clear,
/// the block is 0 and the count is 0 [OFW-4.93 lv1 a:0x453E40-0x453E50]
/// [OFW-4.93 lv1 a:0x4527D0-0x452800]. lv1 sets payload byte 0 to the
/// request's payload byte 1, and bytes 1 and 4 to 7 to 0
/// [OFW-4.93 lv1 a:0x44DF2C-0x44DF5C].
fn first_boot_reply() -> ([u8; 8], Vec<u8>) {
    let rows = capture_rows("reply.tsv");
    let syscon_word = word(&rows, "reply2048", MANAGER + 2048) >> 32;
    assert_eq!(
        syscon_word & 1,
        0,
        "the block holds system controller bytes"
    );
    let count: u16 = 0;
    let c = count.to_be_bytes();
    ([1, 0, c[0], c[1], 0, 0, 0, 0], vec![0; usize::from(count)])
}

#[test]
fn the_service_0x0a_reply_is_the_first_boot_reply_that_the_captured_word_gives() {
    let reply = game_os_system_manager_reply("4.93", GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE, 1)
        .expect("a source gives the reply");
    let (payload, trailing) = first_boot_reply();
    assert_eq!(reply.payload, payload);
    assert_eq!(reply.trailing, trailing.as_slice());
}

/// The kernel stores the count of its last 0x0A reply at +2024 of the
/// object [OFW-4.93 lv2_kernel a:0x8000000000261ae4-0x8000000000261b04].
#[test]
fn the_console_kept_count_is_the_1536_of_a_store_above_the_init_s_largest_count() {
    let rows = capture_rows("stored.tsv");
    assert_eq!(rows.len(), 7);
    assert_eq!(word(&rows, "toc", 0x8000_0000_0000_3000), KERNEL_TOC);
    assert_eq!(word(&rows, "manager", MANAGER_SLOT), MANAGER);
    let kept = word(&rows, "count2024", MANAGER + 2024);
    assert_eq!(kept, 1536);
    assert!(kept > SYSTEM_MANAGER_INIT_STORED_MAX);
}
