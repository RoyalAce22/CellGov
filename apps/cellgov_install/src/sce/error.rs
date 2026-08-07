/// Why SCE/SELF parsing or decryption failed.
#[derive(Debug, thiserror::Error)]
pub enum SceError {
    /// Buffer is too small for a fixed-size structure.
    #[error("SCE: {what} too small ({got} bytes, need {need})")]
    TooSmall {
        /// Name of the structure that was being read.
        what: &'static str,
        /// Bytes available in the input buffer.
        got: usize,
        /// Bytes required to read the structure.
        need: usize,
    },
    /// SCE container magic mismatch.
    #[error("SCE: bad magic 0x{got:08x}")]
    BadMagic {
        /// Magic word actually read at file offset 0.
        got: u32,
    },
    /// No APP key registered for the SELF revision.
    #[error("SCE: no APP key for SELF revision 0x{revision:04x}")]
    NoAppKey {
        /// SELF revision (low 15 bits of `revision_flags`) for which no APP key is known.
        revision: u16,
    },
    /// SELF's ELF header offset is outside the buffer.
    #[error("SCE: {what} offset out of range")]
    HeaderOffsetOutOfRange {
        /// Name of the SELF sub-header whose offset escaped the buffer.
        what: &'static str,
    },
    /// Inner ELF header magic word is not `\x7fELF`.
    #[error("SCE: inner ELF header has bad magic 0x{got:08x}")]
    InnerElfBadMagic {
        /// Magic word read at the inner ELF header offset.
        got: u32,
    },
    /// ELF EI_CLASS is not ELFCLASS64.
    #[error("SCE: SELF ELF header is not ELFCLASS64 (EI_CLASS=0x{got:02x})")]
    BadElfClass {
        /// `EI_CLASS` byte read from the inner ELF header.
        got: u8,
    },
    /// ELF64 header field size disagrees with the architectural constant.
    /// `e_phentsize` must be 0x38 (size of `Elf64_Phdr`); `e_shentsize`
    /// must be 0x40 (size of `Elf64_Shdr`).
    #[error("SCE: inner ELF {what} = 0x{got:04x}, expected 0x{expected:04x}")]
    BadElfEntSize {
        /// Name of the entsize field that disagreed (`e_phentsize` /
        /// `e_shentsize`).
        what: &'static str,
        /// Value read from the inner ELF header.
        got: u16,
        /// Architectural constant for ELF64.
        expected: u16,
    },
    /// AES-256-CBC key-envelope decrypt failed.
    #[error("SCE: AES-256-CBC decrypt failed")]
    AesCbcDecryptFailed,
    /// Key-envelope padding did not decrypt to zero (likely wrong ERK/RIV).
    #[error("SCE: MetadataKeyEnvelope padding validation failed (wrong key?)")]
    KeyEnvelopePadding,
    /// Decrypted metadata directory is shorter than its header.
    #[error("SCE: decrypted metadata too small for header")]
    MetadataTooSmall,
    /// Metadata directory headers extend past the directory buffer.
    #[error("SCE: metadata headers truncated: need {needed} bytes, have {have}")]
    MetadataHeadersTruncated {
        /// Bytes required by the section + key-table layout the directory header claims.
        needed: usize,
        /// Bytes actually present in the decrypted directory buffer.
        have: usize,
    },
    /// Section's encrypted payload extends past the file.
    #[error("SCE: section {index} extends past file end")]
    SectionPastFile {
        /// Zero-based section index that escapes the input buffer.
        index: usize,
    },
    /// Section's key/iv slot index is outside the data-keys table.
    #[error("SCE: section {index} key/iv index out of range")]
    SectionKeyIvIndexOutOfRange {
        /// Zero-based section index with the bad slot index.
        index: usize,
    },
    /// Unknown encryption_kind in section header.
    #[error(
        "SCE: section {index} has unknown encryption_kind {got} (expected 1=plain or 3=aes128-ctr)"
    )]
    UnknownEncryptionKind {
        /// Zero-based section index whose `encryption_kind` was not recognized.
        index: usize,
        /// Raw `encryption_kind` value read from the section descriptor.
        got: u32,
    },
    /// zlib decompress failed for a section.
    #[error("SCE: zlib decompress failed for section {index}: {source}")]
    ZlibDecompress {
        /// Zero-based section index whose decompression failed.
        index: usize,
        /// Underlying `flate2` error.
        #[source]
        source: std::io::Error,
    },
    /// Unknown compression_kind in section header.
    #[error("SCE: section {index} has unknown compression_kind {got} (expected 1=none or 2=zlib)")]
    UnknownCompressionKind {
        /// Zero-based section index whose `compression_kind` was not recognized.
        index: usize,
        /// Raw `compression_kind` value read from the section descriptor.
        got: u32,
    },
    /// Section's program_segment_index >= e_phnum.
    #[error("SCE: section program_segment_index {prog_idx} >= e_phnum {e_phnum}")]
    SectionProgramIndexOutOfRange {
        /// `program_segment_index` claimed by the section descriptor.
        prog_idx: usize,
        /// ELF program-header count read from the inner ELF.
        e_phnum: usize,
    },
    /// Section size disagrees with phdr p_filesz.
    #[error("SCE: section for program segment {prog_idx} has {got} bytes but phdr p_filesz is {expected}")]
    SectionSizeMismatch {
        /// Program-header index the mismatched section targets.
        prog_idx: usize,
        /// Decrypted+decompressed section length.
        got: usize,
        /// `p_filesz` declared by the program header.
        expected: usize,
    },
    /// Section would write past the reconstructed ELF buffer.
    #[error("SCE: section for program segment {prog_idx} (offset 0x{offset:x}, size 0x{size:x}) exceeds reconstructed ELF size 0x{elf_size:x}")]
    SectionPastReconstructedElf {
        /// Program-header index the offending section targets.
        prog_idx: usize,
        /// `p_offset` of the destination program segment.
        offset: usize,
        /// `p_filesz` of the destination program segment.
        size: usize,
        /// Total size of the reconstructed ELF buffer.
        elf_size: usize,
    },
    /// Reconstructed ELF has bad magic.
    #[error("SCE: reconstructed ELF has bad magic 0x{got:08x}")]
    ReconstructedBadMagic {
        /// Magic word read from the reconstructed ELF at offset 0.
        got: u32,
    },
    /// Decrypted package contained no usable section.
    #[error("SCE: no usable section found in decrypted package")]
    NoUsableSection,
    /// NPDRM klicensee lookup returned `None` for the named title.
    #[error("SCE: no RAP/klicensee for NPDRM title {content_id}")]
    NoRapForNpdrmTitle {
        /// `content_id` from the NPD supplemental header.
        content_id: String,
    },
    /// NPDRM license value is not 1, 2, or 3.
    #[error("SCE: NPDRM license value {got} is not 1, 2, or 3")]
    NpdrmBadLicense {
        /// Raw `license` field (u32 BE on the wire).
        got: u32,
    },
    /// SELF carries the debug/fself flag (high bit of `revision_flags`);
    /// the NPDRM decrypt path does not handle unencrypted SELFs.
    #[error("SCE: SELF is flagged debug/fself (revision_flags=0x{revision_flags:04x}); unencrypted SELFs are not in scope for the NPDRM decrypt path")]
    DebugSelfUnsupported {
        /// Raw `revision_flags` field from the SCE container header.
        revision_flags: u16,
    },
}
