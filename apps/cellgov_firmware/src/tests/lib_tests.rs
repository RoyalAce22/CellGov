//! On-disk layout sizes of the PUP and SCE header structs.

use super::*;

#[test]
fn pup_header_size_is_0x30() {
    assert_eq!(std::mem::size_of::<pup::PupHeader>(), 0x30);
}

#[test]
fn pup_entry_size_is_0x20() {
    assert_eq!(std::mem::size_of::<pup::PupFileEntry>(), 0x20);
}

#[test]
fn sce_header_size_is_0x20() {
    assert_eq!(std::mem::size_of::<sce::SceContainerHeader>(), 0x20);
}

#[test]
fn metadata_info_size_is_0x40() {
    assert_eq!(std::mem::size_of::<sce::MetadataKeyEnvelope>(), 0x40);
}

#[test]
fn metadata_header_size_is_0x20() {
    assert_eq!(std::mem::size_of::<sce::EncryptedMetadataDirectory>(), 0x20);
}

#[test]
fn metadata_section_header_size_is_0x30() {
    assert_eq!(std::mem::size_of::<sce::EncryptedSectionDescriptor>(), 0x30);
}
