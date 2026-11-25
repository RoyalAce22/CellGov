//! The reply to the 4.93 kernel's first dispatcher manager request, held
//! to the console capture of the kernel's cache object.

use super::*;

/// The kernel's r2 [OFW-4.93 lv2_kernel a:0x8000000000003000].
const KERNEL_TOC: u64 = 0x8000_0000_0034_fbb0;

/// The TOC slot r2 + 1040 that holds the cache object's address
/// [OFW-4.93 lv2_kernel a:0x800000000022a778].
const CACHE_SLOT: u64 = KERNEL_TOC + 1040;

/// The cache object's address in the image's TOC slot.
const CACHE: u64 = 0x8000_0000_0047_4ae0;

/// The capture's rows as (field, address, value). Both reads of each
/// doubleword must agree.
fn capture() -> Vec<(String, u64, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(DISPATCHER_REPLY_CAPTURE)
        .join("peek/cech20-cex-493/aim_cache.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    let hex = |text: &str| {
        u64::from_str_radix(text.trim_start_matches("0x"), 16)
            .unwrap_or_else(|e| panic!("{name}: {text}: {e}"))
    };
    lines
        .map(|line| {
            let row: Vec<&str> = line.split('\t').collect();
            assert_eq!(row.len(), 6, "{name}: {line}");
            assert_eq!(
                (row[2], row[3], row[4]),
                ("8", "2", "yes"),
                "{name}: {line}"
            );
            (row[1].to_string(), hex(row[0]), hex(row[5]))
        })
        .collect()
}

/// The value of `field` in `rows`. The row's address must be `address`.
fn word(rows: &[(String, u64, u64)], field: &str, address: u64) -> u64 {
    let (_, at, value) = rows
        .iter()
        .find(|(name, ..)| name == field)
        .unwrap_or_else(|| panic!("no row for {field}"));
    assert_eq!(*at, address, "{field}");
    *value
}

/// The 24 bytes at +0 of the cache object, from the capture.
fn cache_bytes() -> [u8; 24] {
    let rows = capture();
    assert_eq!(rows.len(), 5);
    assert_eq!(word(&rows, "toc", 0x8000_0000_0000_3000), KERNEL_TOC);
    assert_eq!(word(&rows, "cache", CACHE_SLOT), CACHE);
    let mut bytes = [0u8; 24];
    for (n, field) in ["cache0", "cache8", "cache16"].into_iter().enumerate() {
        let at = 8 * n;
        let value = word(&rows, field, CACHE + at as u64);
        bytes[at..at + 8].copy_from_slice(&value.to_be_bytes());
    }
    bytes
}

fn first_reply() -> &'static GameOsDispatcherReply {
    game_os_dispatcher_reply(
        "4.93",
        GAME_OS_FIRST_DISPATCHER_SERVICE,
        GAME_OS_FIRST_DISPATCHER_REQUEST,
    )
    .expect("a source gives the reply")
}

/// The getter copies the 16 data bytes to +4 of the cache
/// [OFW-4.93 lv2_kernel a:0x800000000022aaf0-0x800000000022ab0c].
#[test]
fn the_reply_s_data_bytes_are_the_bytes_the_kernel_keeps_at_4_of_its_cache() {
    let cache = cache_bytes();
    assert_eq!(cache[4..20], first_reply().data);
}

/// The getter sets the byte at +0 of the cache to 1 only for status 0
/// [OFW-4.93 lv2_kernel a:0x800000000022aad0]
/// [OFW-4.93 lv2_kernel a:0x800000000022ab10-0x800000000022ab14].
#[test]
fn the_kernel_s_kept_flag_shows_status_0_which_the_reply_gives() {
    let cache = cache_bytes();
    assert_eq!(cache[0], 1);
    assert_eq!(first_reply().status, 0);
}
