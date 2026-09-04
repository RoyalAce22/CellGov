//! ISO9660 (ECMA-119) reader for PS3 BD-ROM images.
//!
//! PS3 discs are UDF-Bridge images, so either filesystem reaches the
//! same content. This reader takes the ISO9660 side and parses no UDF
//! structures. It covers the volume descriptor set at sector 16, the
//! Primary and optional Joliet descriptors, and recursive directory
//! records. Input is a decrypted image; output is a file tree of
//! bounds-checked extent references that a consumer resolves against
//! the image one file at a time ([`IsoEntry::extent_slices`] /
//! [`IsoEntry::read_data`]), because a BD-DL image's content does not
//! fit in host memory.

/// ISO9660 logical sector size.
pub(crate) const SECTOR: usize = 2048;
/// Standard identifier offset within a volume descriptor (after the
/// 1-byte type), and the sector where the descriptor set begins.
pub(crate) const VDS_START_SECTOR: usize = 16;
/// Volume descriptor type: Primary Volume Descriptor.
const VD_PRIMARY: u8 = 1;
/// Volume descriptor type: Supplementary (Joliet) Volume Descriptor.
const VD_SUPPLEMENTARY: u8 = 2;
/// Volume descriptor type: Volume Descriptor Set terminator.
const VD_TERMINATOR: u8 = 255;
/// Byte offset of the root directory record within a PVD/SVD.
const ROOT_RECORD_OFFSET: usize = 156;
/// Fixed directory-record header length preceding the name.
const DIR_RECORD_FIXED: usize = 33;
/// Directory-record flag bit: entry is a directory.
const FLAG_DIRECTORY: u8 = 0b0000_0010;
/// Directory-record flag bit: entry has further extents.
const FLAG_MULTI_EXTENT: u8 = 0b1000_0000;
/// Recursion-depth guard against a malformed self-referential tree.
const MAX_DEPTH: usize = 64;

/// Whether an extracted entry is a file or a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoEntryKind {
    /// A regular file; [`IsoEntry::extents`] locate its bytes.
    File,
    /// A directory; [`IsoEntry::extents`] is empty.
    Directory,
}

/// One extracted ISO entry: its root-relative path and, for files, the
/// extents holding its bytes in the image the entry was read from.
#[derive(Debug, Clone)]
pub struct IsoEntry {
    /// Root-relative path, `/`-separated (e.g. `PS3_GAME/USRDIR/EBOOT.BIN`).
    pub path: String,
    /// File or directory.
    pub kind: IsoEntryKind,
    /// File extents as `(start_sector, byte_size)` in content order,
    /// bounds-checked against the source image by [`read_iso`]. Empty
    /// for a directory. A file above the ~4 GiB single-extent ceiling
    /// carries one pair per extent section.
    pub extents: Vec<(u32, u32)>,
}

impl IsoEntry {
    /// Resolve the extents into ordered byte slices of `image`.
    ///
    /// # Errors
    ///
    /// [`IsoError::ExtentOutOfBounds`] when an extent escapes `image`
    /// -- unreachable for an entry [`read_iso`] produced over the same
    /// image, which already validated every extent during the walk.
    pub fn extent_slices<'i>(&self, image: &'i [u8]) -> Result<Vec<&'i [u8]>, IsoError> {
        self.extents
            .iter()
            .map(|&(start, size)| {
                let range = extent_range(start, size, image.len()).ok_or_else(|| {
                    IsoError::ExtentOutOfBounds {
                        path: self.path.clone(),
                        sector: start,
                        size,
                        len: image.len(),
                    }
                })?;
                Ok(&image[range])
            })
            .collect()
    }

    /// Concatenate the entry's bytes out of `image` into one owned
    /// buffer. Sized for headers and metadata files; a content file can
    /// exceed host memory, so bulk consumers should stream
    /// [`Self::extent_slices`] instead.
    ///
    /// # Errors
    ///
    /// Same as [`Self::extent_slices`].
    pub fn read_data(&self, image: &[u8]) -> Result<Vec<u8>, IsoError> {
        let slices = self.extent_slices(image)?;
        let mut out = Vec::with_capacity(slices.iter().map(|s| s.len()).sum());
        for s in slices {
            out.extend_from_slice(s);
        }
        Ok(out)
    }
}

/// Why reading an ISO9660 image failed.
#[derive(Debug, thiserror::Error)]
pub enum IsoError {
    /// Image is shorter than the volume-descriptor set start.
    #[error("ISO too small (got {len} bytes, need at least {min})", min = (VDS_START_SECTOR + 1) * SECTOR)]
    TooSmall {
        /// Observed image length.
        len: usize,
    },
    /// No `CD001` standard identifier at sector 16.
    #[error("not an ISO9660 image: missing CD001 at sector 16")]
    NotIso,
    /// The descriptor set ran off the image with no terminator.
    #[error("unterminated volume descriptor set")]
    UnterminatedVds,
    /// No Primary or Joliet descriptor before the terminator.
    #[error("ISO has no primary/Joliet volume descriptor")]
    NoRootDescriptor,
    /// A directory record was truncated against the image bounds.
    #[error("ISO directory record at 0x{pos:x} is truncated")]
    RecordTruncated {
        /// Absolute byte offset of the record.
        pos: usize,
    },
    /// A directory or file extent escapes the image.
    #[error("ISO extent for {path:?} [sector {sector}, +0x{size:x}] escapes image 0x{len:x}")]
    ExtentOutOfBounds {
        /// Owning entry path.
        path: String,
        /// Extent start sector.
        sector: u32,
        /// Extent byte size.
        size: u32,
        /// Image length.
        len: usize,
    },
    /// An interleaved file (PS3 discs do not use interleaving).
    #[error("ISO entry {name:?} uses interleaving (unit 0x{unit:x}, gap 0x{gap:x}), unsupported")]
    InterleavedFile {
        /// Entry name.
        name: String,
        /// `file_unit_size`.
        unit: u8,
        /// `interleave` gap.
        gap: u8,
    },
    /// Directory nesting exceeded the depth cap (malformed image).
    #[error("ISO directory nesting exceeds {max}", max = MAX_DEPTH)]
    DepthExceeded,
    /// A directory record declares an Extended Attribute Record. PS3
    /// discs emit none; carving past it is unimplemented, so reject
    /// rather than misread the EAR bytes as the start of file content.
    #[error(
        "ISO record at 0x{pos:x} has an Extended Attribute Record (len {ear_len}), unsupported"
    )]
    UnsupportedExtendedAttributes {
        /// Absolute byte offset of the record.
        pos: usize,
        /// Declared EAR length, in logical blocks.
        ear_len: u8,
    },
    /// A Joliet (UCS-2BE) name had an odd byte length.
    #[error("ISO Joliet name at 0x{pos:x} has an odd byte length")]
    MalformedJolietName {
        /// Absolute byte offset of the owning record.
        pos: usize,
    },
    /// A name was not valid UTF-8 (ISO9660) or UTF-16 (Joliet); for an
    /// oracle, an undecodable identifier is corrupt input, not a name.
    #[error("ISO name at 0x{pos:x} is not decodable")]
    UndecodableName {
        /// Absolute byte offset of the owning record.
        pos: usize,
    },
    /// An identifier decoded to no name at all, or to the `.` / `..`
    /// that ECMA-119 records only as the one-byte `(00)` / `(01)`
    /// identifiers. Neither can name a path component.
    #[error("ISO identifier at 0x{pos:x} decodes to {decoded:?}, which names no entry")]
    NotAnEntryName {
        /// Absolute byte offset of the owning record.
        pos: usize,
        /// The decoded identifier, after the reader removes the version
        /// field and the empty-extension separator.
        decoded: String,
    },
    /// A directory spanned more than one extent. PS3 directories are
    /// single-extent; multi-extent directory carving is unimplemented.
    #[error("ISO directory {path:?} spans multiple extents, unsupported")]
    MultiExtentDirectory {
        /// Owning directory path.
        path: String,
    },
    /// Two records in one directory carry the same name without being a
    /// multi-extent continuation (malformed, or an Associated File).
    #[error("ISO directory has a duplicate entry {path:?}")]
    DuplicateName {
        /// The duplicated path.
        path: String,
    },
}

/// Joliet UCS-2 escape sequences (ECMA-119 / Joliet spec): a
/// Supplementary Volume Descriptor is Joliet only if its Escape
/// Sequences field begins with one of these.
const JOLIET_ESCAPES: [&[u8]; 3] = [b"%/@", b"%/C", b"%/E"];

/// Join a `/`-separated `prefix` with a child `name` (the root prefix
/// is empty, so the child name stands alone).
fn join_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

/// Byte range of the extent `(start, size)` inside an image of
/// `image_len` bytes, or `None` when it escapes the image or its
/// offset does not fit `usize`. The multiply is checked because on a
/// 32-bit host `start * SECTOR` wraps for any start sector at or
/// above 2^21 (a 4 GiB image), and a wrapped base would pass the
/// end-bound test and alias the wrong bytes with no error.
fn extent_range(start: u32, size: u32, image_len: usize) -> Option<std::ops::Range<usize>> {
    let base = (start as usize).checked_mul(SECTOR)?;
    let end = base.checked_add(size as usize)?;
    (end <= image_len).then_some(base..end)
}

fn read_le_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(
        data[off..off + 4]
            .try_into()
            .expect("invariant: caller bounds-checked this 4-byte read"),
    )
}

/// One parsed directory record: identity plus its (first) extent.
struct DirRecord {
    name: String,
    is_dir: bool,
    has_more_extents: bool,
    extents: Vec<(u32, u32)>, // (start_sector, byte_size)
}

/// The chosen root descriptor: its root-directory extent and whether
/// names are Joliet UCS-2.
struct RootInfo {
    root_sector: u32,
    root_size: u32,
    ucs2: bool,
}

/// Read a decrypted ISO9660 image into its file tree (root's children,
/// recursively), each file as bounds-checked extents into `image`.
/// Prefers Joliet names when present.
pub fn read_iso(image: &[u8]) -> Result<Vec<IsoEntry>, IsoError> {
    if image.len() < (VDS_START_SECTOR + 1) * SECTOR {
        return Err(IsoError::TooSmall { len: image.len() });
    }
    // CD001 at sector 16, byte 1 -- the same identifier
    // `container::sniff` routes on, re-checked here because a caller
    // may hand this reader an image it never sniffed.
    if &image[VDS_START_SECTOR * SECTOR + 1..VDS_START_SECTOR * SECTOR + 6] != b"CD001" {
        return Err(IsoError::NotIso);
    }

    let root = walk_volume_descriptors(image)?;

    let mut out = Vec::new();
    walk_directory(
        image,
        root.root_sector,
        root.root_size,
        root.ucs2,
        "",
        &mut out,
        0,
    )?;
    Ok(out)
}

/// Walk the volume descriptor set from sector 16. Selection is by
/// priority, not iteration order: a verified-Joliet Supplementary
/// descriptor wins, else the Primary descriptor. A Supplementary
/// descriptor whose Escape Sequences field is not a known Joliet
/// sequence is ignored (its names are not UCS-2BE). The walk stops at
/// the terminator.
fn walk_volume_descriptors(image: &[u8]) -> Result<RootInfo, IsoError> {
    let mut primary: Option<RootInfo> = None;
    let mut joliet: Option<RootInfo> = None;
    let mut sector = VDS_START_SECTOR;
    loop {
        let base = sector * SECTOR;
        if base + SECTOR > image.len() {
            return Err(IsoError::UnterminatedVds);
        }
        let descriptor_type = image[base];
        if descriptor_type == VD_TERMINATOR {
            break;
        }
        match descriptor_type {
            VD_PRIMARY => primary = Some(read_root_info(image, base, false)?),
            VD_SUPPLEMENTARY if is_joliet_svd(image, base) => {
                joliet = Some(read_root_info(image, base, true)?);
            }
            _ => {}
        }
        sector += 1;
    }
    joliet.or(primary).ok_or(IsoError::NoRootDescriptor)
}

/// Read the root-directory extent out of a PVD/SVD at `base`.
fn read_root_info(image: &[u8], base: usize, ucs2: bool) -> Result<RootInfo, IsoError> {
    let rec = parse_dir_record(image, base + ROOT_RECORD_OFFSET, ucs2)?
        .ok_or(IsoError::NoRootDescriptor)?;
    let (start, size) = rec.extents[0];
    Ok(RootInfo {
        root_sector: start,
        root_size: size,
        ucs2,
    })
}

/// Whether the Supplementary descriptor at `base` is Joliet: its Escape
/// Sequences field (32 bytes at offset 88) begins with a Joliet UCS-2
/// escape sequence. `base + SECTOR` is already bounds-checked by the
/// caller, so the 32-byte read is in range.
fn is_joliet_svd(image: &[u8], base: usize) -> bool {
    let esc = &image[base + 88..base + 120];
    JOLIET_ESCAPES.iter().any(|seq| esc.starts_with(seq))
}

/// Parse the directory record at `pos`. Returns `Ok(None)` for a
/// zero-length record (sector padding). Bounds-checked against the
/// image.
fn parse_dir_record(image: &[u8], pos: usize, ucs2: bool) -> Result<Option<DirRecord>, IsoError> {
    if pos >= image.len() {
        return Err(IsoError::RecordTruncated { pos });
    }
    let entry_length = image[pos] as usize;
    if entry_length == 0 {
        return Ok(None);
    }
    if pos + DIR_RECORD_FIXED > image.len() || pos + entry_length > image.len() {
        return Err(IsoError::RecordTruncated { pos });
    }

    // Extended Attribute Record length (offset 1, in logical blocks).
    // A nonzero EAR shifts the file-data start past the EAR blocks and
    // is excluded from the data length; PS3 discs emit none, so reject
    // rather than silently read EAR bytes as content.
    let ear_len = image[pos + 1];
    if ear_len != 0 {
        return Err(IsoError::UnsupportedExtendedAttributes { pos, ear_len });
    }

    let start_sector = read_le_u32(image, pos + 2);
    let file_size = read_le_u32(image, pos + 10);
    let flags = image[pos + 25];
    let file_unit_size = image[pos + 26];
    let interleave = image[pos + 27];
    let name_len = image[pos + 32] as usize;

    let name_start = pos + DIR_RECORD_FIXED;
    if name_start + name_len > pos + entry_length || name_start + name_len > image.len() {
        return Err(IsoError::RecordTruncated { pos });
    }
    let name_bytes = &image[name_start..name_start + name_len];

    let is_dir = flags & FLAG_DIRECTORY != 0;
    let has_more_extents = flags & FLAG_MULTI_EXTENT != 0;
    let name = decode_name(name_bytes, ucs2, pos)?;

    if (file_unit_size != 0 || interleave != 0) && name != "." && name != ".." {
        return Err(IsoError::InterleavedFile {
            name,
            unit: file_unit_size,
            gap: interleave,
        });
    }

    Ok(Some(DirRecord {
        name,
        is_dir,
        has_more_extents,
        extents: vec![(start_sector, file_size)],
    }))
}

/// Decode a directory-record identifier into the name it stands for,
/// per ECMA-119's identifier grammar:
///
/// - the `(00)` and `(01)` directory identifiers are `.` and `..`,
/// - Joliet big-endian UCS-2 when `ucs2`, else the bytes as UTF-8
///   (ECMA-119 d-characters are ASCII),
/// - strip the version field ([`strip_version`]),
/// - strip the separator before an empty extension.
///
/// # Errors
///
/// - [`IsoError::MalformedJolietName`] on an odd byte length under
///   `ucs2`.
/// - [`IsoError::UndecodableName`] when the bytes are not valid UTF-16
///   (Joliet) or UTF-8.
/// - [`IsoError::NotAnEntryName`] when the strips leave the identifier
///   empty, or when it spells out `.` or `..`. ECMA-119 gives every
///   identifier at least one character, and writes the two directory
///   specials only as `(00)` and `(01)`.
fn decode_name(bytes: &[u8], ucs2: bool, pos: usize) -> Result<String, IsoError> {
    if bytes == [0] {
        return Ok(".".to_string());
    }
    if bytes == [1] {
        return Ok("..".to_string());
    }
    let mut name = if ucs2 {
        if !bytes.len().is_multiple_of(2) {
            return Err(IsoError::MalformedJolietName { pos });
        }
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(u16::from_be_bytes)
            .collect();
        String::from_utf16(&units).map_err(|_| IsoError::UndecodableName { pos })?
    } else {
        std::str::from_utf8(bytes)
            .map_err(|_| IsoError::UndecodableName { pos })?
            .to_string()
    };
    if let Some(stem) = strip_version(&name) {
        name = stem.to_string();
    }
    // Spelled out, either would alias the `(00)` / `(01)` records the
    // walk drops.
    if name == "." || name == ".." {
        return Err(IsoError::NotAnEntryName { pos, decoded: name });
    }
    // An identifier carries the separator even when the extension is
    // empty, so a trailing one is not part of the name.
    if let Some(stem) = name.strip_suffix('.') {
        name = stem.to_string();
    }
    if name.is_empty() {
        return Err(IsoError::NotAnEntryName { pos, decoded: name });
    }
    Ok(name)
}

/// The ECMA-119 version field: `;` then the digits of a version in
/// `1..=32767`. Returns the identifier without it, or `None` when the
/// tail is not a version field and so belongs to the name.
fn strip_version(name: &str) -> Option<&str> {
    let (stem, version) = name.rsplit_once(';')?;
    if version.is_empty() || version.len() > 5 || !version.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    (1..=32767)
        .contains(&version.parse::<u32>().ok()?)
        .then_some(stem)
}

/// Recursively walk a directory extent, emitting its children (and
/// their subtrees) into `out` with `/`-joined paths.
fn walk_directory(
    image: &[u8],
    dir_sector: u32,
    dir_size: u32,
    ucs2: bool,
    prefix: &str,
    out: &mut Vec<IsoEntry>,
    depth: usize,
) -> Result<(), IsoError> {
    if depth > MAX_DEPTH {
        return Err(IsoError::DepthExceeded);
    }
    let base = (dir_sector as usize) * SECTOR;
    let end = base
        .checked_add(dir_size as usize)
        .filter(|&e| e <= image.len())
        .ok_or_else(|| IsoError::ExtentOutOfBounds {
            path: if prefix.is_empty() {
                "/".to_string()
            } else {
                prefix.to_string()
            },
            sector: dir_sector,
            size: dir_size,
            len: image.len(),
        })?;

    let mut records: Vec<DirRecord> = Vec::new();
    let mut pos = base;
    while pos < end {
        if image[pos] == 0 {
            // Records never span a sector; skip padding to the next.
            let next_sector = (pos / SECTOR) + 1;
            pos = next_sector * SECTOR;
            continue;
        }
        let entry_length = image[pos] as usize;
        let rec = parse_dir_record(image, pos, ucs2)?;
        pos += entry_length;
        let Some(rec) = rec else { continue };
        if rec.name == "." || rec.name == ".." {
            continue;
        }
        // Merge a continuation extent into a prior same-name record
        // still expecting more extents (large multi-extent files).
        if let Some(prev) = records
            .iter_mut()
            .rev()
            .find(|r| r.name == rec.name && r.has_more_extents)
        {
            prev.extents.extend(rec.extents);
            prev.has_more_extents = rec.has_more_extents;
            continue;
        }
        // A same-name record that is not an open multi-extent
        // continuation is an undetected ambiguity (two entries collapse
        // to one path); reject rather than emit both.
        if records.iter().any(|r| r.name == rec.name) {
            return Err(IsoError::DuplicateName {
                path: join_path(prefix, &rec.name),
            });
        }
        records.push(rec);
    }

    for rec in records {
        let path = join_path(prefix, &rec.name);
        if rec.is_dir {
            if rec.extents.len() != 1 {
                return Err(IsoError::MultiExtentDirectory { path });
            }
            out.push(IsoEntry {
                path: path.clone(),
                kind: IsoEntryKind::Directory,
                extents: Vec::new(),
            });
            let (sector, size) = rec.extents[0];
            walk_directory(image, sector, size, ucs2, &path, out, depth + 1)?;
        } else {
            // Validate every extent here so the entry's bounds are
            // proven against this image before any consumer resolves it.
            for &(start, size) in &rec.extents {
                extent_range(start, size, image.len()).ok_or_else(|| {
                    IsoError::ExtentOutOfBounds {
                        path: path.clone(),
                        sector: start,
                        size,
                        len: image.len(),
                    }
                })?;
            }
            out.push(IsoEntry {
                path,
                kind: IsoEntryKind::File,
                extents: rec.extents,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/iso_tests.rs"]
mod tests;
