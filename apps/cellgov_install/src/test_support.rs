//! Synthetic-fixture builders shared across the crate's unit tests,
//! plus the progress sinks the install tests assert against. Compiled
//! only under `cfg(test)`; the emitters that encrypt ride with the
//! `decrypt` feature.

use std::path::Path;

use crate::keys::KeyVault;
use crate::progress::{Phase, ProgressSink};

/// Records the phase sequence and the completion flag.
#[derive(Default)]
pub struct RecordingReporter {
    phases: std::sync::Mutex<Vec<u8>>,
    finished: std::sync::atomic::AtomicBool,
}

impl RecordingReporter {
    /// The phase codes reported so far, in order.
    pub fn phases(&self) -> Vec<u8> {
        self.phases.lock().unwrap().clone()
    }

    /// Whether the install reported completion.
    #[cfg_attr(not(feature = "decrypt"), allow(dead_code))]
    pub fn finished(&self) -> bool {
        self.finished.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl ProgressSink for RecordingReporter {
    fn phase(&self, code: u8) {
        self.phases.lock().unwrap().push(code);
    }
    fn totals(&self, _files: usize, _bytes: u64) {}
    fn preset_done(&self, _amount: u64) {}
    fn item_started(&self, _path: &str) {}
    fn advanced(&self, _delta: u64) {}
    fn item_finished(&self) {}
    fn finished(&self) {
        self.finished
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The codes a [`RecordingReporter`] stores for `phases`.
pub fn codes(phases: &[Phase]) -> Vec<u8> {
    phases.iter().map(|p| p.code()).collect()
}

/// A vault of made-up values: enough for every synthetic decrypt path,
/// and nothing a real container opens under. It holds:
///
/// - every scalar slot;
/// - one APP and one NPDRM keyset at revision 0x0001;
/// - one LV2 keyset for firmware 3.55;
/// - one SCE package keyset.
#[cfg_attr(not(feature = "decrypt"), allow(dead_code))]
pub fn synthetic_vault() -> KeyVault {
    fn rep(byte: u8, len: usize) -> String {
        format!("{byte:02x}").repeat(len)
    }
    let toml = format!(
        "pup_hmac = \"{}\"\npkg_aes = \"{}\"\nnp_klic_key = \"{}\"\nnp_klic_free = \"{}\"\n\
         rap_key = \"{}\"\nrap_pbox = \"000102030405060708090a0b0c0d0e0f\"\n\
         rap_e1 = \"{}\"\nrap_e2 = \"{}\"\n\
         [[scepkg]]\nerk = \"{}\"\nriv = \"{}\"\n\
         [[app]]\nrevision = 0x0001\nerk = \"{}\"\nriv = \"{}\"\n\
         [[npdrm]]\nrevision = 0x0001\nerk = \"{}\"\nriv = \"{}\"\n\
         [[lv2]]\nversion = \"3.55\"\nerk = \"{}\"\nriv = \"{}\"\n",
        rep(0x51, 64),
        rep(0x52, 16),
        rep(0x53, 16),
        rep(0x54, 16),
        rep(0x55, 16),
        rep(0x57, 16),
        rep(0x58, 16),
        rep(0x61, 32),
        rep(0x62, 16),
        rep(0x71, 32),
        rep(0x72, 16),
        rep(0x81, 32),
        rep(0x82, 16),
        rep(0x91, 32),
        rep(0x92, 16),
    );
    KeyVault::parse(Path::new("synthetic-keys.toml"), toml.as_bytes())
        .expect("the synthetic vault is well-formed")
}

pub use cellgov_testkit::param_sfo::build_param_sfo;

/// One item in a synthetic PKG.
#[cfg(feature = "decrypt")]
pub struct PkgItem {
    /// Package-relative name.
    pub name: String,
    /// Raw `PKGEntry::type` (low byte selects file/dir/EDAT/SDAT).
    pub raw_type: u32,
    /// File bytes (empty for a directory).
    pub data: Vec<u8>,
}

/// A regular-file item with the given raw type.
#[cfg(feature = "decrypt")]
pub fn pkg_file(name: &str, raw_type: u32, data: &[u8]) -> PkgItem {
    PkgItem {
        name: name.to_string(),
        raw_type,
        data: data.to_vec(),
    }
}

#[cfg(feature = "decrypt")]
fn align16(n: usize) -> usize {
    n.div_ceil(16) * 16
}

/// Build a full retail PKG (data_offset 0x80) carrying `items`,
/// encrypted under `keys`' PKG AES key (CTR is its own inverse, so the
/// production decrypt is the encryptor).
#[cfg(feature = "decrypt")]
pub fn build_pkg(keys: &KeyVault, klic: &[u8; 16], title_id: &str, items: &[PkgItem]) -> Vec<u8> {
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
    crate::pkg::ctr_decrypt(
        keys.pkg_aes().expect("the synthetic vault holds a PKG key"),
        klic,
        &mut region,
    );
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
/// so any decrypt attempt fails.
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

/// Build a USTAR archive of regular files, terminated by the all-zero
/// block `tar::parse` stops at.
///
/// The header carries the fields the parser reads: name, octal size,
/// magic, and a regular-file type flag. The checksum stays zero, which
/// the parser does not check.
///
/// # Panics
///
/// On a name longer than the 100-byte USTAR name field.
#[cfg_attr(not(feature = "decrypt"), allow(dead_code))]
pub fn build_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
    const BLOCK: usize = 512;
    let mut out = Vec::new();
    for (name, data) in entries {
        let mut header = [0u8; BLOCK];
        assert!(
            name.len() <= 100,
            "build_tar: name {name:?} is {} bytes; the USTAR name field holds 100 and this \
             emitter writes no prefix field",
            name.len()
        );
        header[..name.len()].copy_from_slice(name.as_bytes());
        // Size: 11 octal digits then a NUL, at the POSIX offset.
        let size = format!("{:011o}\0", data.len());
        header[0x7C..0x7C + size.len()].copy_from_slice(size.as_bytes());
        header[0x9C] = b'0';
        header[0x101..0x106].copy_from_slice(b"ustar");
        out.extend_from_slice(&header);
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(BLOCK), 0);
    }
    out.extend_from_slice(&[0u8; BLOCK]);
    out
}

/// Build a PUP whose entry table names `entries` by id.
///
/// The emitter records each payload's HMAC-SHA1 under the vault's PUP
/// key, so `validate_hashes` accepts it. It copies the payloads
/// verbatim: a caller that needs a payload read past the hash gate
/// supplies the bytes that gate expects.
#[cfg(feature = "decrypt")]
pub fn build_pup(keys: &KeyVault, image_version: u64, entries: &[(u64, &[u8])]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    use sha1::Sha1;

    let header_len = 0x30 + entries.len() * 0x40;
    let mut payload = Vec::new();
    let mut offsets = Vec::with_capacity(entries.len());
    for (_, data) in entries {
        offsets.push(header_len + payload.len());
        payload.extend_from_slice(data);
    }

    let mut out = Vec::with_capacity(header_len + payload.len());
    out.extend_from_slice(b"SCEUF\0\0\0");
    out.extend_from_slice(&1u64.to_be_bytes());
    out.extend_from_slice(&image_version.to_be_bytes());
    out.extend_from_slice(&(entries.len() as u64).to_be_bytes());
    out.extend_from_slice(&(header_len as u64).to_be_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    for (i, (entry_id, data)) in entries.iter().enumerate() {
        out.extend_from_slice(&entry_id.to_be_bytes());
        out.extend_from_slice(&(offsets[i] as u64).to_be_bytes());
        out.extend_from_slice(&(data.len() as u64).to_be_bytes());
        out.extend_from_slice(&[0u8; 8]);
    }
    let pup_key = keys.pup_hmac().expect("the synthetic vault has a PUP key");
    for (i, (_, data)) in entries.iter().enumerate() {
        let mut mac =
            Hmac::<Sha1>::new_from_slice(pup_key).expect("HMAC-SHA1 takes a key of any length");
        mac.update(data);
        out.extend_from_slice(&(i as u64).to_be_bytes());
        out.extend_from_slice(&mac.finalize().into_bytes());
        out.extend_from_slice(&[0u8; 4]);
    }
    out.extend_from_slice(&payload);
    out
}

/// Build a decrypted CoreOS image whose file table names `entries` in
/// order, each payload placed after the table.
///
/// The header spells the shape retail images carry: a format word of
/// 1, the count, a zero word, and the image length.
///
/// # Panics
///
/// On a name longer than the 32-byte name field.
pub fn build_core_os_image(entries: &[(&str, &[u8])]) -> Vec<u8> {
    const HEADER: usize = 0x10;
    const ENTRY: usize = 0x30;
    let table_end = HEADER + entries.len() * ENTRY;
    let mut payload = Vec::new();
    let mut table = Vec::with_capacity(table_end - HEADER);
    for (name, data) in entries {
        assert!(
            name.len() <= 0x20,
            "build_core_os_image: name {name:?} is {} bytes; the field holds 32",
            name.len()
        );
        let offset = table_end + payload.len();
        payload.extend_from_slice(data);
        table.extend_from_slice(&(offset as u64).to_be_bytes());
        table.extend_from_slice(&(data.len() as u64).to_be_bytes());
        let mut field = [0u8; 0x20];
        field[..name.len()].copy_from_slice(name.as_bytes());
        table.extend_from_slice(&field);
    }
    let len = table_end + payload.len();
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&(len as u32).to_be_bytes());
    out.extend_from_slice(&table);
    out.extend_from_slice(&payload);
    out
}

/// Build an SCE package `sce::decrypt_package` opens under `keys`'
/// first SCE package keyset, carrying `payload` as its one plaintext,
/// uncompressed section.
///
/// The key envelope is AES-256-CBC under the keyset's ERK/RIV, with
/// the all-zero section key and IV inside; the metadata directory is
/// AES-128-CTR under that zero key, which is its own inverse.
#[cfg(feature = "decrypt")]
pub fn build_scepkg(keys: &KeyVault, payload: &[u8]) -> Vec<u8> {
    use aes::cipher::{BlockEncryptMut, KeyIvInit, StreamCipher};

    const METADATA_OFFSET: usize = 0x20;
    const ENVELOPE_OFFSET: usize = METADATA_OFFSET + 0x20;
    const DIRECTORY_OFFSET: usize = ENVELOPE_OFFSET + 0x40;
    const DIRECTORY_LEN: usize = 0x20 + 0x30;
    const HEADER_SIZE: usize = DIRECTORY_OFFSET + DIRECTORY_LEN;
    const PAYLOAD_OFFSET: usize = 0x100;

    let key = keys
        .scepkg_keys()
        .expect("the synthetic vault holds a package keyset")
        .next()
        .expect("at least one package keyset");

    // Plaintext envelope: zero section key, zero padding, zero IV.
    let mut envelope = [0u8; 0x40];
    cbc::Encryptor::<aes::Aes256>::new((&key.erk).into(), (&key.riv).into())
        .encrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut envelope, 0x40)
        .expect("0x40 bytes is a whole number of blocks");

    let mut directory = vec![0u8; DIRECTORY_LEN];
    directory[0x0C..0x10].copy_from_slice(&1u32.to_be_bytes()); // section_count
    let row = 0x20;
    directory[row..row + 8].copy_from_slice(&(PAYLOAD_OFFSET as u64).to_be_bytes());
    directory[row + 8..row + 0x10].copy_from_slice(&(payload.len() as u64).to_be_bytes());
    directory[row + 0x20..row + 0x24].copy_from_slice(&1u32.to_be_bytes()); // plaintext
    directory[row + 0x2C..row + 0x30].copy_from_slice(&1u32.to_be_bytes()); // uncompressed
    ctr::Ctr128BE::<aes::Aes128>::new(&[0u8; 16].into(), &[0u8; 16].into())
        .apply_keystream(&mut directory);

    let mut data = vec![0u8; PAYLOAD_OFFSET + payload.len()];
    data[0..4].copy_from_slice(&cellgov_ps3_abi::format::sce::SCE_MAGIC);
    data[12..16].copy_from_slice(&(METADATA_OFFSET as u32).to_be_bytes());
    data[16..24].copy_from_slice(&(HEADER_SIZE as u64).to_be_bytes());
    data[24..32].copy_from_slice(&(payload.len() as u64).to_be_bytes());
    data[ENVELOPE_OFFSET..DIRECTORY_OFFSET].copy_from_slice(&envelope);
    data[DIRECTORY_OFFSET..HEADER_SIZE].copy_from_slice(&directory);
    data[PAYLOAD_OFFSET..].copy_from_slice(payload);
    data
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
