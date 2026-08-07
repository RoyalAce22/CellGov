//! Synthetic-fixture builders shared across the crate's unit tests:
//! a minimal PARAM.SFO emitter and a retail-PKG emitter. Compiled
//! only under `cfg(test)`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use aes::cipher::{BlockEncrypt, KeyInit};
use cellgov_ps3_abi::sce::PKG_AES_KEY;

/// `format::string` tag.
const SFO_FMT_STRING: u16 = 0x0204;

static SCRATCH_SEQ: AtomicU32 = AtomicU32::new(0);

/// A fresh, empty scratch directory unique to this process + call, for
/// install/uninstall filesystem tests.
pub fn scratch() -> PathBuf {
    let n = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cellgov_game_test_{}_{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Build a minimal PARAM.SFO holding the given string entries (the
/// keys the installer reads). Layout matches the real format: `\0PSF`
/// magic, version 0x101, a 16-byte index per entry, a 4-byte-aligned
/// key table, then the data table.
pub fn build_param_sfo(entries: &[(&str, &str)]) -> Vec<u8> {
    let header_len = 0x14usize;
    let index_len = 0x10usize;

    let mut key_table = Vec::new();
    let mut key_offs = Vec::new();
    for (k, _) in entries {
        key_offs.push(key_table.len() as u16);
        key_table.extend_from_slice(k.as_bytes());
        key_table.push(0);
    }
    while key_table.len() % 4 != 0 {
        key_table.push(0);
    }

    let mut data_table = Vec::new();
    let mut recs: Vec<(u16, u32, u32, u32)> = Vec::new(); // key_off, len, max, data_off
    for (i, (_, v)) in entries.iter().enumerate() {
        let data_off = data_table.len() as u32;
        let mut b = v.as_bytes().to_vec();
        b.push(0);
        let l = b.len() as u32;
        data_table.extend_from_slice(&b);
        recs.push((key_offs[i], l, l, data_off));
    }

    let n = entries.len();
    let off_key_table = (header_len + n * index_len) as u32;
    let off_data_table = off_key_table + key_table.len() as u32;

    let mut buf = Vec::new();
    buf.extend_from_slice(&[0x00, b'P', b'S', b'F']);
    buf.extend_from_slice(&0x0101u32.to_le_bytes());
    buf.extend_from_slice(&off_key_table.to_le_bytes());
    buf.extend_from_slice(&off_data_table.to_le_bytes());
    buf.extend_from_slice(&(n as u32).to_le_bytes());
    for (key_off, len, max, data_off) in &recs {
        buf.extend_from_slice(&key_off.to_le_bytes());
        buf.extend_from_slice(&SFO_FMT_STRING.to_le_bytes());
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&max.to_le_bytes());
        buf.extend_from_slice(&data_off.to_le_bytes());
    }
    buf.extend_from_slice(&key_table);
    buf.extend_from_slice(&data_table);
    buf
}

/// One item in a synthetic PKG.
pub struct PkgItem {
    /// Package-relative name.
    pub name: String,
    /// Raw `PKGEntry::type` (low byte selects file/dir/EDAT/SDAT).
    pub raw_type: u32,
    /// File bytes (empty for a directory).
    pub data: Vec<u8>,
}

/// A regular-file item with the given raw type.
pub fn pkg_file(name: &str, raw_type: u32, data: &[u8]) -> PkgItem {
    PkgItem {
        name: name.to_string(),
        raw_type,
        data: data.to_vec(),
    }
}

fn align16(n: usize) -> usize {
    n.div_ceil(16) * 16
}

/// CTR keystream XOR (symmetric encrypt/decrypt) over a PKG data
/// region, mirroring the production decrypt.
fn pkg_ctr(klic: &[u8; 16], region: &mut [u8]) {
    let cipher = aes::Aes128::new_from_slice(&PKG_AES_KEY).expect("PKG_AES_KEY is 16 bytes");
    let mut counter = u128::from_be_bytes(*klic);
    for block in region.chunks_mut(16) {
        let mut ks = counter.to_be_bytes();
        cipher.encrypt_block((&mut ks).into());
        for (c, k) in block.iter_mut().zip(ks.iter()) {
            *c ^= *k;
        }
        counter = counter.wrapping_add(1);
    }
}

/// Build a full retail PKG (data_offset 0x80) carrying `items`.
pub fn build_pkg(klic: &[u8; 16], title_id: &str, items: &[PkgItem]) -> Vec<u8> {
    let n = items.len();
    let table_len = n * 0x20;

    let mut blob = Vec::new();
    let mut placed: Vec<(u32, u32, u64, u64)> = Vec::new();
    for it in items {
        let name_off = (table_len + blob.len()) as u32;
        let name_bytes = it.name.as_bytes();
        blob.extend_from_slice(name_bytes);
        blob.resize(align16(blob.len()), 0);
        let (file_off, file_size) = if it.data.is_empty() && (it.raw_type & 0xFF) == 4 {
            (0u64, 0u64)
        } else {
            let off = (table_len + blob.len()) as u64;
            blob.extend_from_slice(&it.data);
            blob.resize(align16(blob.len()), 0);
            (off, it.data.len() as u64)
        };
        placed.push((name_off, name_bytes.len() as u32, file_off, file_size));
    }

    let mut region = vec![0u8; table_len];
    for (i, it) in items.iter().enumerate() {
        let (name_off, name_size, file_off, file_size) = placed[i];
        let rec = i * 0x20;
        region[rec..rec + 4].copy_from_slice(&name_off.to_be_bytes());
        region[rec + 4..rec + 8].copy_from_slice(&name_size.to_be_bytes());
        region[rec + 8..rec + 16].copy_from_slice(&file_off.to_be_bytes());
        region[rec + 16..rec + 24].copy_from_slice(&file_size.to_be_bytes());
        region[rec + 24..rec + 28].copy_from_slice(&it.raw_type.to_be_bytes());
    }
    region.extend_from_slice(&blob);

    let data_offset: u64 = 0x80;
    pkg_ctr(klic, &mut region);
    let data_size = region.len() as u64;
    let pkg_size = data_offset + data_size;

    let mut buf = vec![0u8; data_offset as usize];
    buf[0..4].copy_from_slice(&[0x7F, b'P', b'K', b'G']);
    buf[0x04..0x06].copy_from_slice(&0x8000u16.to_be_bytes());
    buf[0x06..0x08].copy_from_slice(&0x0001u16.to_be_bytes());
    buf[0x14..0x18].copy_from_slice(&(n as u32).to_be_bytes());
    buf[0x18..0x20].copy_from_slice(&pkg_size.to_be_bytes());
    buf[0x20..0x28].copy_from_slice(&data_offset.to_be_bytes());
    buf[0x28..0x30].copy_from_slice(&data_size.to_be_bytes());
    let tid = title_id.as_bytes();
    buf[0x30..0x30 + tid.len()].copy_from_slice(tid);
    buf[0x70..0x80].copy_from_slice(klic);
    buf.extend_from_slice(&region);
    buf
}

/// Build a minimal NPDRM-classifiable EBOOT header: an SCE container
/// with a type-3 (NPDRM) supplemental record carrying `license` and
/// `content_id`, enough for `find_npd_header_info` to classify it.
///
/// The bytes are not a real encryptable SELF (revision 0 has no key),
/// so a decrypt attempt fails -- which is exactly what the install
/// decrypt-proof gate observes when fed a synthetic EBOOT.
pub fn build_npdrm_eboot_header(license: u32, content_id: &str) -> Vec<u8> {
    const SUPP_OFF: usize = 0x80;
    const BODY_LEN: usize = 0x80;
    const RECORD_SIZE: usize = 0x10 + BODY_LEN; // header + NPD body
    let mut buf = vec![0u8; SUPP_OFF + RECORD_SIZE];
    // SCE container magic "SCE\0".
    buf[0..4].copy_from_slice(&0x5343_4500u32.to_be_bytes());
    // Extended header: supplemental chain offset (0x58) and size (0x60).
    buf[0x58..0x60].copy_from_slice(&(SUPP_OFF as u64).to_be_bytes());
    buf[0x60..0x68].copy_from_slice(&(RECORD_SIZE as u64).to_be_bytes());
    // One supplemental record: kind 3 (NPDRM), then its size.
    buf[SUPP_OFF..SUPP_OFF + 4].copy_from_slice(&3u32.to_be_bytes());
    buf[SUPP_OFF + 4..SUPP_OFF + 8].copy_from_slice(&(RECORD_SIZE as u32).to_be_bytes());
    // NPD body: license (BE u32 at +0x08), content-id (+0x10..+0x40).
    let body = SUPP_OFF + 0x10;
    buf[body + 0x08..body + 0x0C].copy_from_slice(&license.to_be_bytes());
    let cid = content_id.as_bytes();
    let n = cid.len().min(0x30);
    buf[body + 0x10..body + 0x10 + n].copy_from_slice(&cid[..n]);
    buf
}

/// ISO9660 logical sector size, exposed for byte-level test tampering.
pub const ISO_SECTOR: usize = 2048;

/// A node in a synthetic ISO tree.
pub enum IsoNode {
    /// A file with a name and bytes.
    File(&'static str, Vec<u8>),
    /// A directory with a name and children.
    Dir(&'static str, Vec<IsoNode>),
}

struct IsoBuilt {
    sector: u32,
    size: u32,
    placements: Vec<(u32, Vec<u8>)>,
}

/// Emit one ISO9660 directory record (both-endian sector/size, `;1`
/// added by the caller for files).
fn iso_dir_record(name: &[u8], sector: u32, size: u32, is_dir: bool) -> Vec<u8> {
    let mut entry_len = 33 + name.len();
    if entry_len % 2 == 1 {
        entry_len += 1;
    }
    let mut r = vec![0u8; entry_len];
    r[0] = entry_len as u8;
    r[2..6].copy_from_slice(&sector.to_le_bytes());
    r[6..10].copy_from_slice(&sector.to_be_bytes());
    r[10..14].copy_from_slice(&size.to_le_bytes());
    r[14..18].copy_from_slice(&size.to_be_bytes());
    r[25] = if is_dir { 0x02 } else { 0x00 };
    r[28..30].copy_from_slice(&1u16.to_le_bytes());
    r[30..32].copy_from_slice(&1u16.to_be_bytes());
    r[32] = name.len() as u8;
    r[33..33 + name.len()].copy_from_slice(name);
    r
}

fn iso_build_node(node: &IsoNode, next_sector: &mut u32) -> IsoBuilt {
    match node {
        IsoNode::File(_, data) => {
            let sectors = data.len().div_ceil(ISO_SECTOR).max(1) as u32;
            let sector = *next_sector;
            *next_sector += sectors;
            let mut bytes = data.clone();
            bytes.resize(sectors as usize * ISO_SECTOR, 0);
            IsoBuilt {
                sector,
                size: data.len() as u32,
                placements: vec![(sector, bytes)],
            }
        }
        IsoNode::Dir(_, children) => {
            let my_sector = *next_sector;
            *next_sector += 1;
            let built: Vec<(&IsoNode, IsoBuilt)> = children
                .iter()
                .map(|c| (c, iso_build_node(c, next_sector)))
                .collect();

            let mut records = Vec::new();
            records.extend(iso_dir_record(&[0], my_sector, ISO_SECTOR as u32, true));
            records.extend(iso_dir_record(&[1], my_sector, ISO_SECTOR as u32, true));
            let mut placements = Vec::new();
            for (child, b) in &built {
                let (name, is_dir) = match child {
                    IsoNode::File(n, _) => (format!("{n};1"), false),
                    IsoNode::Dir(n, _) => (n.to_string(), true),
                };
                records.extend(iso_dir_record(name.as_bytes(), b.sector, b.size, is_dir));
                placements.extend(b.placements.clone());
            }
            assert!(
                records.len() <= ISO_SECTOR,
                "test dir extent must fit one sector"
            );
            records.resize(ISO_SECTOR, 0);

            let mut all = vec![(my_sector, records)];
            all.extend(placements);
            IsoBuilt {
                sector: my_sector,
                size: ISO_SECTOR as u32,
                placements: all,
            }
        }
    }
}

/// Build a full single-PVD ISO9660 image whose root directory holds
/// `roots` (PVD at sector 16, terminator at sector 17).
pub fn build_iso(roots: Vec<IsoNode>) -> Vec<u8> {
    let root = IsoNode::Dir("", roots);
    let mut next_sector = 18u32;
    let built = iso_build_node(&root, &mut next_sector);

    let mut image = vec![0u8; next_sector as usize * ISO_SECTOR];

    let pvd = 16 * ISO_SECTOR;
    image[pvd] = 1;
    image[pvd + 1..pvd + 6].copy_from_slice(b"CD001");
    image[pvd + 6] = 1;
    let root_rec = iso_dir_record(&[0], built.sector, built.size, true);
    image[pvd + 156..pvd + 156 + root_rec.len()].copy_from_slice(&root_rec);

    let term = 17 * ISO_SECTOR;
    image[term] = 255;
    image[term + 1..term + 6].copy_from_slice(b"CD001");

    for (sector, bytes) in &built.placements {
        let off = *sector as usize * ISO_SECTOR;
        image[off..off + bytes.len()].copy_from_slice(bytes);
    }
    image
}
