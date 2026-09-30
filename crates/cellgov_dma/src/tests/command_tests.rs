//! One case per row of Table 7-6, the commands the rows exempt, and the
//! boundaries each row accepts.

use super::*;

/// A valid 16-byte transfer: aligned local store, matching low bits.
fn transfer() -> MfcParameters {
    MfcParameters {
        lsa: 0x100,
        eah: 0,
        eal: 0x2000,
        size: 16,
        tag: 3,
    }
}

fn check(
    class: MfcCommandClass,
    change: impl FnOnce(&mut MfcParameters),
) -> Result<(), MfcCommandError> {
    let mut params = transfer();
    change(&mut params);
    validate(class, params)
}

#[test]
fn a_valid_transfer_passes() {
    assert_eq!(validate(MfcCommandClass::Transfer, transfer()), Ok(()));
}

/// [CBEA p:57 s:7.2 Table 7-6] Invalid MFC Command Tag: any reserved bit not 0; a DMA command error.
#[test]
fn a_reserved_tag_bit_is_an_invalid_command() {
    for tag in [32, 0x100, u32::MAX] {
        let error = check(MfcCommandClass::Transfer, |p| p.tag = tag).unwrap_err();
        assert_eq!(error, MfcCommandError::ReservedTagBits(tag));
        assert_eq!(error.class(), MfcExceptionClass::InvalidCommand);
    }
    assert_eq!(check(MfcCommandClass::Transfer, |p| p.tag = 31), Ok(()));
}

/// [CBEA p:57 s:7.2 Table 7-6] footnote 2: a command error is reported ahead of an alignment error.
#[test]
fn a_command_error_outranks_an_alignment_error() {
    let error = check(MfcCommandClass::Transfer, |p| {
        p.tag = 32;
        p.size = 3;
    });
    assert_eq!(error, Err(MfcCommandError::ReservedTagBits(32)));
}

/// [CBEA p:57 s:7.2 Table 7-6] Transfer Size Alignment Error: any reserved bit not 0.
#[test]
fn a_reserved_size_bit_is_an_alignment_error() {
    let error = check(MfcCommandClass::Transfer, |p| p.size = 0x8000).unwrap_err();
    assert_eq!(error, MfcCommandError::ReservedSizeBits(0x8000));
    assert_eq!(error.class(), MfcExceptionClass::Alignment);
}

/// [CBEA p:57 s:7.2 Table 7-6] a transfer size greater than 16K bytes.
#[test]
fn a_size_above_16_kb_is_an_alignment_error() {
    assert_eq!(
        check(MfcCommandClass::Transfer, |p| p.size = 0x4010),
        Err(MfcCommandError::SizeTooLarge(0x4010))
    );
    assert_eq!(
        check(MfcCommandClass::Transfer, |p| p.size = 0x4000),
        Ok(())
    );
}

/// [CBEA p:57 s:7.2 Table 7-6] the transfer size is not 1, 2, 4, or 8 bytes or a multiple of 16 bytes.
#[test]
fn a_size_that_is_not_a_power_below_16_or_a_multiple_of_16_is_an_alignment_error() {
    for size in [3, 5, 12, 17, 24] {
        assert_eq!(
            check(MfcCommandClass::Transfer, |p| p.size = size),
            Err(MfcCommandError::SizeUnaligned(size)),
            "size {size}"
        );
    }
}

/// [CBEA p:57 s:7.2 Table 7-6] local-store address bits 31, 30:31, 29:31 and 28:31 must be zero for sizes 2, 4, 8 and multiples of 16.
#[test]
fn a_local_store_address_not_aligned_for_its_size_is_an_alignment_error() {
    for (size, lsa) in [(2, 0x101), (4, 0x102), (8, 0x104), (16, 0x108), (32, 0x108)] {
        assert_eq!(
            check(MfcCommandClass::Transfer, |p| {
                p.size = size;
                p.lsa = lsa;
                p.eal = 0x2000 | (lsa & 0xF);
            }),
            Err(MfcCommandError::LocalStoreUnaligned { lsa, size }),
            "size {size} at 0x{lsa:x}"
        );
    }
    // One byte at an odd address is aligned, and so is no byte at all.
    for size in [0, 1] {
        assert_eq!(
            check(MfcCommandClass::Transfer, |p| {
                p.size = size;
                p.lsa = 0x103;
                p.eal = 0x2003;
            }),
            Ok(()),
            "size {size}"
        );
    }
}

/// [CBEA p:57 s:7.2 Table 7-6] Effective Address Alignment Error: EA bits 60:63 not equal to LSA bits 28:31, for every put and get and for sndsig.
#[test]
fn effective_and_local_store_addresses_with_different_low_bits_are_an_alignment_error() {
    let error = check(MfcCommandClass::Transfer, |p| p.eal = 0x2004);
    assert_eq!(
        error,
        Err(MfcCommandError::AddressLowBitsDiffer {
            lsa: 0x100,
            ea: 0x2004
        })
    );
    let error = check(MfcCommandClass::SendSignal, |p| {
        p.size = 4;
        p.eal = 0x2004;
    });
    assert!(matches!(
        error,
        Err(MfcCommandError::AddressLowBitsDiffer { .. })
    ));
}

/// [CBEA p:57 s:7.2 Table 7-6] the transfer size for a sndsig command is not 4 B.
#[test]
fn a_sndsig_of_any_size_but_4_is_an_alignment_error() {
    assert_eq!(
        check(MfcCommandClass::SendSignal, |p| p.size = 16),
        Err(MfcCommandError::SendSignalSize(16))
    );
    assert_eq!(check(MfcCommandClass::SendSignal, |p| p.size = 4), Ok(()));
}

/// [CBEA p:57 s:7.2 Table 7-6] List Transfer Size, list local-store address and List Address Alignment Errors.
#[test]
fn a_list_checks_its_size_and_both_doubleword_alignments() {
    assert_eq!(
        check(MfcCommandClass::List, |p| p.size = 0x4008),
        Err(MfcCommandError::SizeTooLarge(0x4008))
    );
    assert_eq!(
        check(MfcCommandClass::List, |p| p.size = 0x10000),
        Err(MfcCommandError::ReservedSizeBits(0x10000))
    );
    assert_eq!(
        check(MfcCommandClass::List, |p| p.lsa = 0x104),
        Err(MfcCommandError::LocalStoreUnaligned {
            lsa: 0x104,
            size: 16
        })
    );
    assert_eq!(
        check(MfcCommandClass::List, |p| p.eal = 0x2004),
        Err(MfcCommandError::ListAddressUnaligned(0x2004))
    );
    // A list's addresses need not share their low bits: each element
    // names its own effective address.
    assert_eq!(check(MfcCommandClass::List, |p| p.eal = 0x2008), Ok(()));
}

/// [CBEA p:57 s:7.2 Table 7-6] footnote 1: none of the alignment checks apply to mfcsync, mfceieio, barrier and the atomic commands.
#[test]
fn atomic_and_synchronization_commands_are_not_checked() {
    let bad = MfcParameters {
        lsa: 0x103,
        eah: 0,
        eal: 0x2008,
        size: 0x1_0003,
        tag: u32::MAX,
    };
    for class in [MfcCommandClass::Atomic, MfcCommandClass::Synchronization] {
        assert_eq!(validate(class, bad), Ok(()), "{class:?}");
    }
}

#[test]
fn each_error_has_its_own_code() {
    let errors = [
        MfcCommandError::ReservedTagBits(0),
        MfcCommandError::ReservedSizeBits(0),
        MfcCommandError::SizeTooLarge(0),
        MfcCommandError::SizeUnaligned(0),
        MfcCommandError::SendSignalSize(0),
        MfcCommandError::LocalStoreUnaligned { lsa: 0, size: 0 },
        MfcCommandError::AddressLowBitsDiffer { lsa: 0, ea: 0 },
        MfcCommandError::ListAddressUnaligned(0),
    ];
    let codes: std::collections::BTreeSet<u8> = errors.iter().map(|e| e.code()).collect();
    assert_eq!(codes.len(), errors.len());
}
