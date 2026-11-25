use std::collections::BTreeSet;
use std::path::Path;

use super::*;
use crate::lv1::repository::LIST_CAPTURE;

const PEEK_HEADER: &str = "address\treads\tagree\tdoubleword";
const NODE_HEADER: &str = "fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2";
const LIST_HEADER: &str = "fw\tlpar\tn1\tn2\tn3\tn4\tnode_address\tsplit_order_key\treads\tagree";

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The rows of the file `name` of the capture `dir`, after the header
/// `header`, each split into its `columns` cells.
fn rows(dir: &str, name: &str, header: &str, columns: usize) -> Vec<Vec<String>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir)
        .join(name);
    let shown = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{shown}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(header), "{shown}: header");
    lines
        .map(|line| {
            let row: Vec<String> = line.split('\t').map(str::to_string).collect();
            assert_eq!(row.len(), columns, "{shown}: {line:?}");
            row
        })
        .collect()
}

/// The doublewords of the peek file `name`, each with its address, after
/// a check that both reads of each word agree.
fn peek(name: &str) -> Vec<(u64, u64)> {
    rows(REGISTRATION_CAPTURE, name, PEEK_HEADER, 4)
        .iter()
        .map(|row| {
            assert_eq!((row[1].as_str(), row[2].as_str()), ("2", "yes"), "{row:?}");
            (hex(&row[0]), hex(&row[3]))
        })
        .collect()
}

/// The key, v1 and v2 of each row of the node file `name`.
fn nodes(name: &str) -> Vec<(RepositoryKey, u64, u64)> {
    rows(REGISTRATION_CAPTURE, name, NODE_HEADER, 11)
        .iter()
        .map(|row| {
            assert_eq!((row[0].as_str(), row[1].as_str()), ("4.93", "1"), "{row:?}");
            assert_eq!((row[7].as_str(), row[8].as_str()), ("2", "yes"), "{row:?}");
            let key = RepositoryKey {
                lpar: LPAR_ID_PME,
                n1: hex(&row[2]),
                n2: hex(&row[3]),
                n3: hex(&row[4]),
                n4: hex(&row[5]),
            };
            (key, hex(&row[9]), hex(&row[10]))
        })
        .collect()
}

fn registration_4_93() -> &'static StorageRegistration {
    registration("4.93").expect("the 4.93 registration")
}

/// The TOC slot of the storage bus global holds 0x368388
/// [OFW-4.93 lv1 a:0x353818-0x35381C].
#[test]
fn the_registration_gets_the_storage_bus_object_itself() {
    let words = peek("bus.tsv");
    assert_eq!(words.len(), 3);
    let (bus_global, bus) = words[0];
    assert_eq!(bus_global, 0x0036_8388);
    assert_eq!(words[1], (bus + 128, bus));
}

#[test]
fn the_captured_bus_table_is_each_devices_notify_entry() {
    let words: Vec<u64> = peek("table.tsv").iter().map(|&(_, w)| w).collect();
    let entries: Vec<u64> = registration_4_93()
        .devices
        .iter()
        .flat_map(StorageDevice::notify_entry)
        .collect();
    assert_eq!(words.len(), 20);
    assert_eq!(words, entries);
}

#[test]
fn the_captured_counts_give_five_devices_and_the_num_dev_word() {
    let words: Vec<u64> = peek("counts.tsv").iter().map(|&(_, w)| w).collect();
    let devices = u64::try_from(registration_4_93().devices.len()).expect("a count");
    assert_eq!(words, [devices, devices, registration_4_93().num_dev]);
}

/// The registration reads the node index at +3536
/// [OFW-4.93 lv1 a:0x252504-0x252534], and `blk_size` and `n_blocks` at
/// +3464 and +3472 [OFW-4.93 lv1 a:0x252558-0x25266C].
#[test]
fn each_device_object_holds_its_id_as_the_node_index_and_its_block_words() {
    let words = peek("device_fields.tsv");
    let devices = registration_4_93().devices;
    assert_eq!(words.len(), 4 * devices.len());
    for (device, chunk) in devices.iter().zip(words.chunks(4)) {
        let object = chunk[0].0;
        let at = |offset: u64| {
            let found: Vec<u64> = chunk
                .iter()
                .filter(|&&(a, _)| a == object + offset)
                .map(|&(_, w)| w)
                .collect();
            assert_eq!(found.len(), 1, "device {} +{offset}", device.id);
            found[0]
        };
        assert_eq!(at(3464), device.blk_size, "device {}", device.id);
        assert_eq!(at(3472), device.n_blocks, "device {}", device.id);
        assert_eq!(at(3536), device.id, "device {}", device.id);
    }
}

#[test]
fn each_captured_node_holds_the_value_the_model_gives_after_five_registrations() {
    let reg = registration_4_93();
    let captured: Vec<(RepositoryKey, u64, u64)> = nodes("node.tsv")
        .into_iter()
        .chain(nodes("region.tsv"))
        .collect();
    assert_eq!(captured.len(), 52 + 84);
    for (key, v1, v2) in captured {
        assert_eq!(reg.node(0..5, &key), Some((v1, v2)), "{}", key.path());
    }
}

#[test]
fn the_placeholder_keys_are_the_bus_4_keys_of_the_repository_walk() {
    let walk: BTreeSet<RepositoryKey> = rows(LIST_CAPTURE, "list.tsv", LIST_HEADER, 10)
        .iter()
        .map(|row| RepositoryKey {
            lpar: row[1].parse().unwrap_or_else(|e| panic!("{row:?}: {e}")),
            n1: hex(&row[2]),
            n2: hex(&row[3]),
            n3: hex(&row[4]),
            n4: hex(&row[5]),
        })
        .filter(|key| key.n1 == first_field(b"bus", STORAGE_BUS_INDEX))
        .collect();
    let placeholders: BTreeSet<RepositoryKey> =
        placeholder_nodes().iter().map(|node| node.key).collect();
    assert_eq!(placeholders.len(), placeholder_nodes().len());
    assert_eq!(walk.len(), 290);
    assert_eq!(placeholders, walk);
}

#[test]
fn the_registration_keys_are_keys_of_the_repository_walk() {
    let reg = registration_4_93();
    let placeholders: BTreeSet<RepositoryKey> =
        placeholder_nodes().iter().map(|node| node.key).collect();
    let written: Vec<WrittenNode> = reg
        .devices
        .iter()
        .flat_map(|device| registration_nodes(reg, device))
        .collect();
    assert_eq!(written.len(), 5 * 12 + 4 * 16);
    for node in written {
        assert!(placeholders.contains(&node.key), "{}", node.key.path());
    }
}
