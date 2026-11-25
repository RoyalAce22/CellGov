//! lv1's storage device registration: the repository nodes of lpar 1
//! that lv1 writes for a storage device before it reports the device to
//! the game OS partition.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`, and from the console capture
//! [`REGISTRATION_CAPTURE`]. Its README gives the whole path. In short:
//! - The read method of the notification device calls the registration
//!   0x2524A4 for a game OS record before it reports a device
//!   [OFW-4.93 lv1 a:0x259540-0x25955C].
//! - lv1's device-ready event calls it for each game OS record before
//!   it completes a waiting read [OFW-4.93 lv1 a:0x258F80-0x258F94]
//!   [OFW-4.93 lv1 a:0x259004-0x259040].
//! - The registration first calls a one-time initializer, which writes
//!   the placeholder nodes of [`placeholder_nodes`]
//!   [OFW-4.93 lv1 a:0x25201C-0x2523AC].
//! - Then it writes the nodes of [`registration_nodes`] for the device
//!   [OFW-4.93 lv1 a:0x2524A4-0x2526E4].
//!
//! Each node write overwrites the node, or makes it when lpar 1's
//! repository holds no node of the key [OFW-4.93 lv1 a:0x237BDC-0x237C54].
//!
//! No read covers these parts:
//! - the check 0x2E49E8 that selects the ids of the placeholders; the
//!   capture shows nodes for each id from 0 to 6;
//! - the body of the region access check's ACL comparison, from
//!   [OFW-4.93 lv1 a:0x25B6F0-0x25B7C8]. No path of the registration
//!   enters it.

use super::partition::LPAR_ID_PME;
use super::repository::{field, first_field, RepositoryKey};

/// The console capture of the registration's nodes, its device table
/// and the device objects, relative to the workspace root.
pub const REGISTRATION_CAPTURE: &str = "tests/micro/lv1_storage_registration/peek/cech20-cex-493";

/// The bus index of the storage bus in lpar 1's repository keys.
///
/// lv1 passes 4 as the bus index of each node write
/// [OFW-4.93 lv1 a:0x252530] [OFW-4.93 lv1 a:0x252578].
pub const STORAGE_BUS_INDEX: u64 = 4;

/// v1 of `bus.4.id`, which the registration writes
/// [OFW-4.93 lv1 a:0x252678].
pub const STORAGE_BUS_ID: u64 = 4;

/// v1 of `bus.4.type`, which the registration writes
/// [OFW-4.93 lv1 a:0x25267C].
pub const STORAGE_BUS_TYPE: u64 = 5;

/// v1 of `bus.4.dev.<n>.intr.0`, which the registration writes
/// [OFW-4.93 lv1 a:0x252540].
pub const INTERRUPT_NODE_V1: u64 = 2;

/// v1 of each placeholder `type` node [OFW-4.93 lv1 a:0x252140].
pub const PLACEHOLDER_TYPE: u64 = 32;

/// v2 of each placeholder node that the initializer writes through its
/// varargs key builder [OFW-4.93 lv1 a:0x252170-0x252180].
pub const PLACEHOLDER_V2: u64 = 0xDEAD_BEEF;

/// The number of device ids that the initializer walks: 0 to 6
/// [OFW-4.93 lv1 a:0x252390].
pub const PLACEHOLDER_DEVICES: u64 = 7;

/// The number of placeholder regions of each device: 0 to 7
/// [OFW-4.93 lv1 a:0x252354].
pub const PLACEHOLDER_REGIONS: u64 = 8;

/// One region of a storage device, as `bus.4.dev.<n>.region.<i>` holds
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageRegion {
    /// v1 of `start`.
    pub start: u64,
    /// v1 of `size`.
    pub size: u64,
    /// v1 of `crypto`: 1 when the region's byte at +81 is set
    /// [OFW-4.93 lv1 a:0x25B2A4-0x25B2C0].
    pub crypto: u64,
}

/// One storage device that lv1 reports to the game OS, with the values
/// that its registration writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageDevice {
    /// The device id: +8 of its bus table entry, and the node index.
    pub id: u64,
    /// The device type: +16 of its table entry, and v1 of `type`.
    pub kind: u64,
    /// +24 of its table entry, and v1 of `port`.
    pub port: u64,
    /// v1 of `blk_size`.
    pub blk_size: u64,
    /// v1 of `n_blocks`.
    pub n_blocks: u64,
    /// v1 of `meddling`.
    pub meddling: u64,
    /// v1 of `hosttype`.
    pub hosttype: u64,
    /// The regions that the registration counts, in order.
    pub regions: &'static [StorageRegion],
}

impl StorageDevice {
    /// The four doublewords of the device's entry in the bus table.
    ///
    /// The first doubleword of each captured entry is
    /// [`STORAGE_BUS_ID`].
    pub fn notify_entry(&self) -> [u64; 4] {
        [STORAGE_BUS_ID, self.id, self.kind, self.port]
    }
}

/// The registration of the storage devices at a set of firmware
/// versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageRegistration {
    /// The firmware versions, ascending.
    pub firmware: &'static [&'static str],
    /// The devices of the bus table, in table order.
    pub devices: &'static [StorageDevice],
    /// v1 of `bus.4.num_dev`: the bus word at +3808.
    pub num_dev: u64,
}

impl StorageRegistration {
    /// The device of id `id`, or `None` when the bus table holds no
    /// entry for it.
    pub fn device(&self, id: u64) -> Option<&'static StorageDevice> {
        self.devices.iter().find(|device| device.id == id)
    }

    /// The value of the node `key` after lv1 registered each device of
    /// `registered`, or `None` when no registration write and no
    /// placeholder gives the key.
    ///
    /// The answer is `None` for each key when `registered` holds no id
    /// of the table. The initializer runs in the first registration, so
    /// no source gives a node before it. Other ids in `registered` write
    /// nothing. Two registrations write the same bus nodes with the same
    /// values, so the order of `registered` does not change an answer.
    pub fn node(
        &self,
        registered: impl IntoIterator<Item = u64>,
        key: &RepositoryKey,
    ) -> Option<(u64, u64)> {
        let devices: Vec<&StorageDevice> = registered
            .into_iter()
            .filter_map(|id| self.device(id))
            .collect();
        if devices.is_empty() {
            return None;
        }
        let written = devices
            .into_iter()
            .flat_map(|device| registration_nodes(self, device))
            .find(|node| node.key == *key);
        written
            .or_else(|| {
                placeholder_nodes()
                    .into_iter()
                    .find(|node| node.key == *key)
            })
            .map(|node| (node.v1, node.v2))
    }
}

/// The 4.93 regions, from `region.tsv` of [`REGISTRATION_CAPTURE`].
const REGIONS_DEVICE_0: &[StorageRegion] = &[StorageRegion {
    start: 0,
    size: 0x7FFF_FFFF,
    crypto: 0,
}];
const REGIONS_DEVICE_1: &[StorageRegion] = &[
    StorageRegion {
        start: 0,
        size: 0x8000,
        crypto: 0,
    },
    StorageRegion {
        start: 0x8,
        size: 0x77F8,
        crypto: 0,
    },
    StorageRegion {
        start: 0x7900,
        size: 0x100,
        crypto: 0,
    },
    StorageRegion {
        start: 0x7A00,
        size: 0x400,
        crypto: 0,
    },
];
const REGIONS_DEVICE_2: &[StorageRegion] = REGIONS_DEVICE_0;
const REGIONS_DEVICE_3: &[StorageRegion] = &[
    StorageRegion {
        start: 0,
        size: 0x0DF9_4BB0,
        crypto: 0,
    },
    StorageRegion {
        start: 0x0008_0020,
        size: 0x0DB1_4B90,
        crypto: 0,
    },
    StorageRegion {
        start: 0x0DB9_4BB0,
        size: 0x003F_FFF8,
        crypto: 0,
    },
];
const REGIONS_DEVICE_4: &[StorageRegion] = &[
    StorageRegion {
        start: 0,
        size: 0x8_0000,
        crypto: 0,
    },
    StorageRegion {
        start: 0x8,
        size: 0x75F8,
        crypto: 1,
    },
    StorageRegion {
        start: 0x7800,
        size: 0x6_3E00,
        crypto: 1,
    },
    StorageRegion {
        start: 0x6_B600,
        size: 0x8000,
        crypto: 1,
    },
    StorageRegion {
        start: 0x7_3600,
        size: 0x400,
        crypto: 1,
    },
    StorageRegion {
        start: 0x7_3A00,
        size: 0x2000,
        crypto: 1,
    },
    StorageRegion {
        start: 0x7_7C00,
        size: 0x200,
        crypto: 1,
    },
];

/// The registrations whose values a source gives.
///
/// At 4.93, `table.tsv`, `node.tsv` and `region.tsv` of
/// [`REGISTRATION_CAPTURE`] give each value. The capture reads the nodes
/// after boot. The model assumes that the bus held its five devices, and
/// its word at +3808 held 5, at the game OS's first registration.
pub const REGISTRATIONS: &[StorageRegistration] = &[StorageRegistration {
    firmware: &["4.93"],
    devices: &[
        StorageDevice {
            id: 0,
            kind: 0xFF,
            port: 0,
            blk_size: 0x800,
            n_blocks: 0x7FFF_FFFF,
            meddling: 0,
            hosttype: 0xFF,
            regions: REGIONS_DEVICE_0,
        },
        StorageDevice {
            id: 1,
            kind: 0xFE,
            port: 0,
            blk_size: 0x200,
            n_blocks: 0x8000,
            meddling: 0,
            hosttype: 0xFE,
            regions: REGIONS_DEVICE_1,
        },
        StorageDevice {
            id: 2,
            kind: 5,
            port: 0,
            blk_size: 0x800,
            n_blocks: 0x7FFF_FFFF,
            meddling: 1,
            hosttype: 5,
            regions: REGIONS_DEVICE_2,
        },
        StorageDevice {
            id: 3,
            kind: 0,
            port: 2,
            blk_size: 0x200,
            n_blocks: 0x0DF9_4BB0,
            meddling: 0,
            hosttype: 0,
            regions: REGIONS_DEVICE_3,
        },
        StorageDevice {
            id: 4,
            kind: 14,
            port: 0,
            blk_size: 0x200,
            n_blocks: 0x8_0000,
            meddling: 0,
            hosttype: 0,
            regions: REGIONS_DEVICE_4,
        },
    ],
    num_dev: 5,
}];

/// The registration at the firmware version `firmware`, or `None` when
/// no source gives it.
pub fn registration(firmware: &str) -> Option<&'static StorageRegistration> {
    REGISTRATIONS
        .iter()
        .find(|registration| registration.firmware.contains(&firmware))
}

/// One repository node that lv1 writes, with its two values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrittenNode {
    /// The node's key in lpar 1's repository.
    pub key: RepositoryKey,
    /// The first value.
    pub v1: u64,
    /// The second value.
    pub v2: u64,
}

/// The key `bus.4.<name>` of lpar 1.
const fn bus_key(name: &[u8]) -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_PME,
        n1: first_field(b"bus", STORAGE_BUS_INDEX),
        n2: field(name, 0),
        n3: 0,
        n4: 0,
    }
}

/// The key `bus.4.dev.<device>.<name>.<index>.<sub>` of lpar 1, with
/// `sub` empty when it is `None`.
const fn device_key(device: u64, name: &[u8], index: u64, sub: Option<&[u8]>) -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_PME,
        n1: first_field(b"bus", STORAGE_BUS_INDEX),
        n2: field(b"dev", device),
        n3: field(name, index),
        n4: match sub {
            Some(sub) => field(sub, 0),
            None => 0,
        },
    }
}

const fn node(key: RepositoryKey, v1: u64, v2: u64) -> WrittenNode {
    WrittenNode { key, v1, v2 }
}

/// The three bus nodes that the registration and the initializer write,
/// with `num_dev` as v1 of `bus.4.num_dev`. Both call the entry
/// 0x2380B8. It moves the bus id, the type and `num_dev` into place with
/// v2 = 0, and then branches to the writer
/// [OFW-4.93 lv1 a:0x2380B8-0x2380C4] [OFW-4.93 lv1 a:0x237E8C-0x2380B4].
fn bus_nodes(num_dev: u64) -> [WrittenNode; 3] {
    [
        node(bus_key(b"id"), STORAGE_BUS_ID, 0),
        node(bus_key(b"type"), STORAGE_BUS_TYPE, 0),
        node(bus_key(b"num_dev"), num_dev, 0),
    ]
}

/// The nodes that lv1's registration of `device` writes, in lv1's order.
///
/// The registration writes these nodes:
/// - `id` and `type` [OFW-4.93 lv1 a:0x252504-0x252534]
///   [OFW-4.93 lv1 a:0x23830C-0x23849C];
/// - `intr.0` [OFW-4.93 lv1 a:0x252538-0x25254C]
///   [OFW-4.93 lv1 a:0x2387F8-0x238960];
/// - `region.<i>.id`, `.size`, `.start` and `.crypto` for each region,
///   and then `n_regs` [OFW-4.93 lv1 a:0x25B16C-0x25B34C];
/// - `blk_size`, `n_blocks`, `port`, `meddling` and `hosttype`
///   [OFW-4.93 lv1 a:0x252558-0x25266C];
/// - `bus.4.id`, `bus.4.type` and `bus.4.num_dev`
///   [OFW-4.93 lv1 a:0x252670-0x252680].
///
/// Each node holds v2 = 0.
pub fn registration_nodes(
    registration: &StorageRegistration,
    device: &StorageDevice,
) -> Vec<WrittenNode> {
    let id = device.id;
    let mut nodes = vec![
        node(device_key(id, b"id", 0, None), id, 0),
        node(device_key(id, b"type", 0, None), device.kind, 0),
        node(device_key(id, b"intr", 0, None), INTERRUPT_NODE_V1, 0),
    ];
    for (index, region) in (0u64..).zip(device.regions) {
        nodes.extend([
            node(device_key(id, b"region", index, Some(b"id")), index, 0),
            node(
                device_key(id, b"region", index, Some(b"size")),
                region.size,
                0,
            ),
            node(
                device_key(id, b"region", index, Some(b"start")),
                region.start,
                0,
            ),
            node(
                device_key(id, b"region", index, Some(b"crypto")),
                region.crypto,
                0,
            ),
        ]);
    }
    let regions = u64::try_from(device.regions.len()).unwrap_or(u64::MAX);
    nodes.extend([
        node(device_key(id, b"n_regs", 0, None), regions, 0),
        node(device_key(id, b"blk_size", 0, None), device.blk_size, 0),
        node(device_key(id, b"n_blocks", 0, None), device.n_blocks, 0),
        node(device_key(id, b"port", 0, None), device.port, 0),
        node(device_key(id, b"meddling", 0, None), device.meddling, 0),
        node(device_key(id, b"hosttype", 0, None), device.hosttype, 0),
    ]);
    nodes.extend(bus_nodes(registration.num_dev));
    nodes
}

/// The placeholder nodes that lv1's one-time initializer writes for each
/// device id from 0 to 6, in lv1's order.
///
/// For each id the initializer writes these nodes
/// [OFW-4.93 lv1 a:0x252130-0x25238C]:
/// - `id` = (id, 0), `type` = ([`PLACEHOLDER_TYPE`], 0) and `intr.0` =
///   ([`INTERRUPT_NODE_V1`], 0);
/// - `blk_size`, `n_blocks`, `port`, `meddling` and `hosttype` = (0,
///   [`PLACEHOLDER_V2`]);
/// - for each region i from 0 to 7: `region.<i>.id` = (i,
///   [`PLACEHOLDER_V2`]), and `start`, `size` and `crypto` = (0,
///   [`PLACEHOLDER_V2`]);
/// - `n_regs` = (0, [`PLACEHOLDER_V2`]).
///
/// Then it writes the bus nodes with `num_dev` = 0
/// [OFW-4.93 lv1 a:0x25239C-0x2523AC].
pub fn placeholder_nodes() -> Vec<WrittenNode> {
    let mut nodes = Vec::new();
    for id in 0..PLACEHOLDER_DEVICES {
        nodes.extend([
            node(device_key(id, b"id", 0, None), id, 0),
            node(device_key(id, b"type", 0, None), PLACEHOLDER_TYPE, 0),
            node(device_key(id, b"intr", 0, None), INTERRUPT_NODE_V1, 0),
        ]);
        for name in [
            &b"blk_size"[..],
            b"n_blocks",
            b"port",
            b"meddling",
            b"hosttype",
        ] {
            nodes.push(node(device_key(id, name, 0, None), 0, PLACEHOLDER_V2));
        }
        for index in 0..PLACEHOLDER_REGIONS {
            nodes.push(node(
                device_key(id, b"region", index, Some(b"id")),
                index,
                PLACEHOLDER_V2,
            ));
            for sub in [&b"start"[..], b"size", b"crypto"] {
                nodes.push(node(
                    device_key(id, b"region", index, Some(sub)),
                    0,
                    PLACEHOLDER_V2,
                ));
            }
        }
        nodes.push(node(device_key(id, b"n_regs", 0, None), 0, PLACEHOLDER_V2));
    }
    nodes.extend(bus_nodes(0));
    nodes
}

#[cfg(test)]
#[path = "tests/storage_registration_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/storage_registration_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/storage_placeholder_console_tests.rs"]
mod placeholder_console_tests;
