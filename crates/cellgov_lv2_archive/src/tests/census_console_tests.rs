//! The 4.93 census against the syscall table the reference console runs.

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::lv2::syscall::SYSCALL_TABLE_SLOTS;

/// The slots whose not-implemented stub the console's CFW replaces with
/// a handler of its own.
const CFW_PATCHED_SLOTS: [usize; 8] = [6, 7, 8, 9, 10, 11, 15, 35];

fn workspace_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn read_text(relative: &str) -> String {
    let path = workspace_file(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The probe's payload as big-endian u64 words.
fn console_words() -> Vec<u64> {
    let path =
        workspace_file("tests/micro/lv2_syscall_table_live/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    assert_eq!(frame.len(), 8 + len, "one whole frame");
    frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

fn hex(cell: &str) -> u64 {
    u64::from_str_radix(cell.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The 4.93 census as `(class, handler)` per slot.
fn census_slots() -> Vec<(String, u64)> {
    read_text("docs/lv2/census/fw-4.93.tsv")
        .lines()
        .skip(1)
        .enumerate()
        .map(|(slot, line)| {
            let cells: Vec<&str> = line.split('\t').collect();
            assert_eq!(cells[1], slot.to_string(), "census rows run in slot order");
            (cells[2].to_string(), hex(cells[3]))
        })
        .collect()
}

/// The `#define <name>` value in the probe's source.
fn probe_define(name: &str) -> u64 {
    let source = read_text("tests/micro/lv2_syscall_table_live/ppu/main.c");
    let value = source
        .lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("#define") && words.next() == Some(name))
                .then(|| words.next())
                .flatten()
        })
        .unwrap_or_else(|| panic!("no #define {name}"));
    let value = value.trim_end_matches("ULL");
    if value.starts_with("0x") {
        hex(value)
    } else {
        value
            .parse()
            .unwrap_or_else(|e| panic!("{name} = {value}: {e}"))
    }
}

#[test]
fn the_running_4_93_table_is_the_census_on_every_slot_the_cfw_leaves() {
    let census = census_slots();
    assert_eq!(census.len() as u64, SYSCALL_TABLE_SLOTS);
    assert_eq!(probe_define("TABLE_SLOTS"), SYSCALL_TABLE_SLOTS);
    let kernel_row = read_text("docs/lv2/tables/kernel.tsv")
        .lines()
        .find(|line| line.starts_with("158471fd834f8ea8"))
        .expect("the 4.93 PUP's kernel row")
        .to_string();
    assert_eq!(
        hex(kernel_row.split('\t').nth(2).expect("table_base")),
        probe_define("TABLE_BASE")
    );

    let words = console_words();
    assert_eq!(words.len(), 2 + 2 * census.len());
    assert_eq!(words[1], 0, "every entry lies inside the kernel image");
    let differing: Vec<usize> = (0..census.len())
        .filter(|&slot| words[3 + 2 * slot] != census[slot].1)
        .collect();
    assert_eq!(
        differing, CFW_PATCHED_SLOTS,
        "slots whose handler differs from the census"
    );
    for slot in CFW_PATCHED_SLOTS {
        assert_eq!(
            census[slot].0, "stub",
            "slot {slot} is a stub in the kernel image"
        );
    }
}
