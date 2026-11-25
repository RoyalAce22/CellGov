//! The lv1 state that decides the start of the controller query, held to
//! the console capture of that state after boot.

use super::*;
use crate::lv1::partition::{LPAR_ID_GAME_OS, LPAR_ID_PME};
use crate::lv1::repository::{field, first_field};
use crate::lv1::storage_registration::registration;

/// The TOC slot that the policy check loads G from: r2 + 120
/// [OFW-4.93 lv1 a:0x31D7B0], with lv1's TOC at 0x35A038
/// [OFW-4.93 lv1 a:0x351C6C].
const POLICY_TOC_SLOT: u64 = 0x35_A0B0;

/// The table index that the dispatcher passes in r4
/// [OFW-4.93 lv1 a:0x272FA4].
const POLICY_INDEX: u64 = 1;

/// The bits that the dispatcher passes in r6, which the V of the deciding
/// record must hold [OFW-4.93 lv1 a:0x272FA8]
/// [OFW-4.93 lv1 a:0x31D744-0x31D750].
const POLICY_RIGHT: u64 = 1;

/// The offset in the table of the entry pointer of index 0
/// [OFW-4.93 lv1 a:0x31D434-0x31D43C].
const ENTRY_POINTERS_OFFSET: u64 = 16;

/// The offset in an entry of its byte count, and of its first record
/// [OFW-4.93 lv1 a:0x31D448-0x31D4B0].
const COUNT_OFFSET: u64 = 8;
const RECORDS_OFFSET: u64 = 16;

/// The bytes of a policy record [OFW-4.93 lv1 a:0x31D4A0]
/// [OFW-4.93 lv1 a:0x31D5E4].
const RECORD_BYTES: u64 = 32;

/// The global that holds the partition table pointer, from the TOC slot
/// [OFW-4.93 lv1 a:0x358228].
const PARTITION_TABLE_GLOBAL: u64 = 0x37_3D18;

/// The bucket of lpar 2's `ss.laid` node: the hash 0x1F126966 AND 15
/// [OFW-4.93 lv1 a:0x2C9774-0x2C9798].
const LAID_BUCKET: u64 = 6;

/// The split-order key of lpar 2's `ss.laid` node: the bit-reversed hash
/// of its key, OR 1 [OFW-4.93 lv1 a:0x2E3874-0x2E38D8].
const LAID_SPLIT_ORDER_KEY: u64 = 0x6696_48F9;

/// The global that holds the storage bus object, from the TOC slot at
/// r2 - 23408 [OFW-4.93 lv1 a:0x251340-0x251360]
/// [OFW-4.93 lv1 a:0x3544C8-0x3544CC].
const STORAGE_BUS_GLOBAL: u64 = 0x36_8388;

/// The offset in the storage bus object of device 0's object pointer
/// [OFW-4.93 lv1 a:0x251BF0-0x251C54].
const DEVICES_OFFSET: u64 = 3816;

/// The device whose controller the query asks.
const DEVICE: u64 = 2;

/// The method table of device 2: see the device open capture.
const DEVICE_2_METHODS: u64 = 0x35_E370;

/// The offset in the device object of the tag bitmap
/// [OFW-4.93 lv1 a:0x253990].
const TAGS_OFFSET: u64 = 2680;

/// The offset in the device object of its block size: see the
/// registration capture.
const BLOCK_SIZE_OFFSET: u64 = 3464;

/// The offset in the device object of the controller pointer, and in the
/// controller of its word [OFW-4.93 lv1 a:0x272D20-0x272D28].
const CONTROLLER_OFFSET: u64 = 5688;
const CONTROLLER_WORD_OFFSET: u64 = 16;

/// The controller word with which the dispatcher starts a command
/// [OFW-4.93 lv1 a:0x272E20-0x272E24].
const CONTROLLER_IDLE: u64 = 1;

/// The offset in the device object of the ring index, of slot 0, and the
/// bytes of a slot [OFW-4.93 lv1 a:0x272D2C-0x272D38].
const RING_INDEX_OFFSET: u64 = 7688;
const RING_OFFSET: u64 = 6272;
const SLOT_BYTES: u64 = 88;

/// The offset in a slot of its state word [OFW-4.93 lv1 a:0x272D3C].
const SLOT_STATE_OFFSET: u64 = 8;

/// The slots of the ring [OFW-4.93 lv1 a:0x272D68].
const RING_SLOTS: u64 = 16;

/// The slot state that sends the dispatcher to the start test
/// [OFW-4.93 lv1 a:0x272D40-0x272D44].
const SLOT_READY: u64 = 1;

/// The value of the word at `address` in the capture's file `name`.
///
/// Panics when the two reads of the word disagree.
fn captured(name: &str, address: u64) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(CONTROLLER_QUERY_CAPTURE)
        .join(name);
    let file = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut lines = text.lines();
    let header = lines.next();
    assert!(
        header == Some("address\treads\tagree\tdoubleword")
            || header == Some("address\treads\tagree\tword"),
        "{file}: header {header:?}"
    );
    let hex = |cell: &str| {
        let digits = cell
            .strip_prefix("0x")
            .unwrap_or_else(|| panic!("{file}: {cell} is not 0x hex"));
        u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{file}: {cell}: {e}"))
    };
    let row: Vec<&str> = lines
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.first().map(|cell| hex(cell)) == Some(address))
        .unwrap_or_else(|| panic!("{file}: no row for {address:#x}"));
    assert_eq!(row.len(), 4, "{file}: {address:#x} columns");
    assert_eq!(row[1..3], ["2", "yes"], "{file}: {address:#x}");
    hex(row[3])
}

/// The policy table T: the doubleword at G, which the TOC slot holds
/// [OFW-4.93 lv1 a:0x31D7B0-0x31D7C8].
fn policy_table() -> u64 {
    let global = captured("pointer.tsv", POLICY_TOC_SLOT);
    captured("pointer.tsv", global)
}

/// The dummy node of `bucket` in lpar 1's repository, from the captured
/// walk.
///
/// - The partition table's segment list is at +24, and segment 0's entry
///   array is at +72 of the segment [OFW-4.93 lv1 a:0x2E53F4-0x2E54B8].
/// - The repository is at +56 of lpar 1's partition
///   [OFW-4.93 lv1 a:0x2E5388].
/// - The hash table is at the repository + 8
///   [OFW-4.93 lv1 a:0x2E3D20]. Its segment 0 is at table + 32, and the
///   bucket at segment + 56 + 8 * bucket
///   [OFW-4.93 lv1 a:0x2C97A0-0x2C97B8].
fn lpar_1_bucket(bucket: u64) -> u64 {
    let read = |address| captured("repository.tsv", address);
    let partitions = read(PARTITION_TABLE_GLOBAL);
    let segment = read(read(partitions + 24));
    assert_eq!(read(segment + 48), 8, "the entry size of segment 0");
    let lpar_1 = read(read(segment + 72) + 8 * LPAR_ID_PME);
    let table = read(lpar_1 + 56) + 8;
    read(read(table + 32) + 56 + 8 * bucket)
}

/// v1 of lpar 1's `ss.laid` node for the game OS partition, from the
/// captured walk of the node's bucket.
///
/// A node holds its next pointer at +48, its split-order key at +56,
/// its four key words at +64 and v1 at +104
/// [OFW-4.93 lv1 a:0x2E39B0-0x2E3A00] [OFW-4.93 lv1 a:0x2E3D48]
/// [OFW-4.93 lv1 a:0x2E3E2C-0x2E3E34].
fn game_os_laid() -> u64 {
    let mut node = lpar_1_bucket(LAID_BUCKET);
    for _ in 0..6 {
        if captured("walk_words.tsv", node + 56) == LAID_SPLIT_ORDER_KEY {
            break;
        }
        node = captured("walk_next.tsv", node + 48);
    }
    let key = [64, 72, 80, 88].map(|offset| captured("laid.tsv", node + offset));
    assert_eq!(
        key,
        [first_field(b"ss", 0), field(b"laid", 0), LPAR_ID_GAME_OS, 0]
    );
    captured("laid.tsv", node + 104)
}

/// lv1's policy check of `command` for `partition`, whose key is `id`,
/// over the captured table.
///
/// [OFW-4.93 lv1 a:0x31D3BC-0x31D3E4] [OFW-4.93 lv1 a:0x31D434-0x31D778]
fn policy(partition: u64, id: u64, command: u64) -> i64 {
    let table = policy_table();
    if captured("table.tsv", table) == 0 {
        return if partition == LPAR_ID_PME {
            0
        } else {
            SEND_COMMAND_DENIED_STATUS
        };
    }
    let entry = captured(
        "table.tsv",
        table + ENTRY_POINTERS_OFFSET + 8 * POLICY_INDEX,
    );
    let records = captured("entry.tsv", entry + COUNT_OFFSET) / RECORD_BYTES;
    assert_eq!(records, 32);
    for i in 0..records {
        let record = entry + RECORDS_OFFSET + i * RECORD_BYTES;
        let [x, m, a, v] = [0, 8, 16, 24].map(|offset| captured("records.tsv", record + offset));
        if (a == 0 || a == command) && x & m == id & m {
            return if v & POLICY_RIGHT == POLICY_RIGHT {
                0
            } else {
                SEND_COMMAND_DENIED_STATUS
            };
        }
    }
    SEND_COMMAND_DENIED_STATUS
}

/// The object of device 2: the pointer at +3816 + 8 * 2 of the storage
/// bus object.
fn device_2() -> u64 {
    let bus = captured("bus.tsv", STORAGE_BUS_GLOBAL);
    let device = captured("device.tsv", bus + DEVICES_OFFSET + 8 * DEVICE);
    assert_eq!(captured("object.tsv", device), DEVICE_2_METHODS);
    device
}

/// The dispatcher passes the command id shifted left 32 in r5
/// [OFW-4.93 lv1 a:0x272F94-0x272FA0].
#[test]
fn the_captured_policy_passes_the_controller_query_of_the_game_os_partition() {
    assert_ne!(captured("table.tsv", policy_table()), 0);
    let command = u64::from(CONTROLLER_QUERY_COMMAND) << 32;
    assert_eq!(policy(LPAR_ID_GAME_OS, game_os_laid(), command), 0);
}

#[test]
fn the_captured_ring_slot_and_controller_of_device_2_start_the_query_in_the_call() {
    let device = device_2();
    let controller = captured("object.tsv", device + CONTROLLER_OFFSET);
    assert_eq!(
        captured("controller.tsv", controller + CONTROLLER_WORD_OFFSET),
        CONTROLLER_IDLE
    );
    let state = |slot: u64| {
        captured(
            "ring.tsv",
            device + RING_OFFSET + slot * SLOT_BYTES + SLOT_STATE_OFFSET,
        )
    };
    let index = captured("ring.tsv", device + RING_INDEX_OFFSET);
    assert!(index < RING_SLOTS, "ring index {index}");
    assert_eq!(state(index), SLOT_READY);
    let ready = (0..RING_SLOTS)
        .filter(|&slot| state(slot) == SLOT_READY)
        .count();
    assert_eq!(ready, 16);
}

/// The request function gives the lowest clear bit of the bitmap as the
/// tag [OFW-4.93 lv1 a:0x2539DC-0x253A04].
#[test]
fn device_2_holds_no_captured_request_tag_so_the_query_gets_tag_0() {
    let bitmap = captured("object.tsv", device_2() + TAGS_OFFSET);
    assert_eq!(bitmap, 0);
    let tag = (0..SEND_COMMAND_TAGS).find(|&tag| bitmap & (1 << tag) == 0);
    assert_eq!(tag, Some(0));
}

/// The request method passes no word of the device object at +3464 to
/// the request function [OFW-4.93 lv1 a:0x256D54-0x256D6C].
#[test]
fn the_captured_word_at_3464_of_device_2_is_its_registered_block_size() {
    let device = registration("4.93")
        .and_then(|table| table.device(DEVICE))
        .expect("the 4.93 registration of device 2");
    assert_eq!(device.id, DEVICE);
    assert_eq!(
        captured("object.tsv", device_2() + BLOCK_SIZE_OFFSET),
        device.blk_size
    );
}
