//! The kind of the storage bus's notification requests, held to the
//! console capture of the requests after boot.

use super::*;
use crate::lv1::storage::notify_console_tests::{bus, captured, ENTRY_BYTES, ENTRY_OFFSET};

/// The offset in a request entry of the doubleword whose first byte is
/// the completed byte and whose second byte is the kind byte
/// [OFW-4.93 lv1 a:0x255E4C] [OFW-4.93 lv1 a:0x255E74]
/// [OFW-4.93 lv1 a:0x255E90].
const COMPLETED_OFFSET: u64 = 40;

/// The offset in a request entry of the doubleword that the status
/// method tests before it returns 0 [OFW-4.93 lv1 a:0x255EB8]
/// [OFW-4.93 lv1 a:0x255F04-0x255F18].
const RETURN_WORD_OFFSET: u64 = 32;

/// The game OS partition, which holds tag 1 in the capture.
const GAME_OS_PARTITION: u64 = 2;

/// The tag of the game OS partition's request in the capture.
const GAME_OS_TAG: u64 = 1;

#[test]
fn the_captured_game_os_request_waits_with_the_kind_that_the_status_core_asks_for() {
    let entry = bus() + ENTRY_OFFSET + GAME_OS_TAG * ENTRY_BYTES;
    assert_eq!(captured("entries.tsv", entry), GAME_OS_PARTITION);
    let flags = captured("entries.tsv", entry + COMPLETED_OFFSET).to_be_bytes();
    assert_eq!(flags[0], 0, "the request waits");
    assert_eq!(flags[1], ASYNC_STATUS_KIND);
}

/// The notification write stores 0 at +32 [OFW-4.93 lv1 a:0x253A48-0x253A50],
/// and the completion function does not write it.
#[test]
fn the_captured_game_os_request_holds_the_0_at_32_with_which_the_status_method_returns_0() {
    let entry = bus() + ENTRY_OFFSET + GAME_OS_TAG * ENTRY_BYTES;
    assert_eq!(captured("entries.tsv", entry), GAME_OS_PARTITION);
    assert_eq!(captured("entries.tsv", entry + RETURN_WORD_OFFSET), 0);
}
