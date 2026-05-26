//! RAP -> klicensee derivation and NPDRM SELF decrypt prefix.
//!
//! NPDRM-wrapped SELFs (PSN-HDD titles such as flOw, SSHD) carry an
//! extra AES-128-CBC layer over the SCE metadata-info envelope.
//! Peeling that layer needs a 16-byte klicensee derived from a RAP
//! file the operator ships alongside the title. This module owns
//! that derivation and the NPDRM-prefix decrypt; the post-envelope
//! flow joins [`crate::sce::decrypt_self_to_elf`]'s shared CTR path.
//!
//! The NPDRM RAP-to-klicensee transform and the metadata-info
//! envelope wrapping are not documented in any public Sony spec.
//! The algorithms below are reverse-engineered (constants in
//! [`cellgov_ps3_abi::sce`] carry the bytes; `CLAUDE.local.md`
//! forbids citing the leaked SDK). RPCS3's `rap_to_rif` and
//! `DecryptNPDRM` are the de-facto cross-reference implementations
//! and are named once here as corroboration -- not as the
//! specification.
//!
//! Correctness is anchored on **format self-certification**, not
//! on byte-agreement with any other implementation:
//!
//! - A wrong klicensee or wrong layer key leaves the
//!   [`crate::sce::MetadataKeyEnvelope`] padding bytes non-zero,
//!   tripping [`SceError::KeyEnvelopePadding`].
//! - A wrong section CTR key produces a malformed metadata
//!   directory, tripping
//!   [`SceError::MetadataHeadersTruncated`] or worse.
//! - A correct full decrypt produces a parseable PS3 ELF.
//!
//! The end-to-end test in
//! `apps/cellgov_firmware/tests/parity_decrypted_reference.rs`
//! enforces "decrypts to a parseable ELF" against real titles;
//! that is the correctness gate. The smaller oracle vectors in
//! this file's test module are **localization aids** -- they
//! tell you whether a break is in `rap_to_klic` versus the
//! envelope path versus reassembly -- not independent
//! correctness definitions.
//!
//! Fixture-dependent tests (those that `include_bytes!`
//! operator-supplied RAP and EBOOT files under
//! `tools/rpcs3/dev_hdd0/`) live behind the
//! `npdrm-oracle-vectors` feature. A fresh checkout without
//! those files compiles cleanly by default; CI enables the
//! feature on the host that carries them.

use aes::cipher::{BlockDecrypt, KeyInit};
use cellgov_ps3_abi::sce::{NP_KLIC_FREE, NP_KLIC_KEY, RAP_E1, RAP_E2, RAP_KEY, RAP_PBOX};

use crate::sce::{
    assemble_elf_from_sections, decrypt_envelope, decrypt_sections_from_envelope,
    find_npd_header_info, parse_sce_header, NpdHeaderInfo, SceError,
};

/// Derive the 16-byte intermediate klicensee from a 16-byte RAP.
///
/// One AES-128-ECB-decrypt with `RAP_KEY`, then five rounds of
/// PBOX permutation + E1 XOR + descending cascade + E2
/// borrow-subtraction. The output is what the NPDRM scheme calls
/// the "RIF key" -- an intermediate value that the envelope-peel
/// step further ECB-decrypts with `NP_KLIC_KEY` to produce the
/// layer key. Splitting the two steps mirrors the RPCS3
/// implementation (`rap_to_rif`); the format itself doesn't name
/// the boundary.
///
/// Pure function: same RAP in, same klic out on any host. No
/// console identity, no act.dat, no IDPS.
#[must_use]
pub fn rap_to_klic(rap: &[u8; 16]) -> [u8; 16] {
    // Initial stage: aes_crypt_cbc(AES_DECRYPT, 0x10, iv=zeros, rap,
    // key). For a single 16-byte block with IV=0, CBC-decrypt
    // equals ECB-decrypt; use the simpler primitive.
    let cipher = aes::Aes128::new_from_slice(&RAP_KEY).expect("RAP_KEY is 16 bytes");
    let mut key = [0u8; 16];
    key.copy_from_slice(rap);
    cipher.decrypt_block((&mut key).into());

    for _round in 0..5 {
        // Phase 1: XOR each PBOX-indexed byte with the matching E1 byte.
        for &p in RAP_PBOX.iter() {
            let p = p as usize;
            key[p] ^= RAP_E1[p];
        }
        // Phase 2: cascade XOR through descending PBOX pairs.
        for i in (1..16).rev() {
            let p = RAP_PBOX[i] as usize;
            let pp = RAP_PBOX[i - 1] as usize;
            key[p] ^= key[pp];
        }
        // Phase 3: borrow-propagating subtraction sweep using E2.
        let mut o: u8 = 0;
        for &pi in RAP_PBOX.iter() {
            let p = pi as usize;
            let kc = key[p].wrapping_sub(o);
            let ec2 = RAP_E2[p];
            if o != 1 || kc != 0xFF {
                o = u8::from(kc < ec2);
                key[p] = kc.wrapping_sub(ec2);
            } else {
                // C has three arms. The final `else` (which writes
                // `kc` unchanged) is unreachable: reaching the
                // else-if chain requires `o == 1 && kc == 0xFF`, so
                // `else if (kc == 0xFF)` always fires. Only the
                // live `kc - ec2` arm (no `o` update) survives the
                // collapse.
                key[p] = kc.wrapping_sub(ec2);
            }
        }
    }

    key
}

/// Derive the NPDRM layer key from a klicensee by AES-128-ECB-
/// decrypting with `NP_KLIC_KEY`.
///
/// The layer key is the AES-128 key that decrypts the NPDRM-wrapped
/// metadata-info envelope. RPCS3 corroborates this step in
/// `DecryptNPDRM`.
fn klicensee_to_layer_key(klicensee: &[u8; 16]) -> [u8; 16] {
    let cipher = aes::Aes128::new_from_slice(&NP_KLIC_KEY).expect("NP_KLIC_KEY is 16 bytes");
    let mut layer_key = [0u8; 16];
    layer_key.copy_from_slice(klicensee);
    cipher.decrypt_block((&mut layer_key).into());
    layer_key
}

/// Decrypt an NPDRM-wrapped SELF using the supplied 16-byte
/// klicensee and reconstruct the plaintext ELF.
///
/// Full chain: reject debug SELFs (format gate; see
/// [`SceError::DebugSelfUnsupported`]), derive the NPDRM layer
/// key, AES-128-CBC-decrypt the envelope (NPDRM peel), AES-256-
/// CBC-decrypt the envelope with the title's APP key (APP peel),
/// then run the shared CTR-based section decrypt + ELF
/// reassembly. RPCS3's `DecryptNPDRM` + `LoadMetadata` chain is
/// the cross-reference implementation.
///
/// The klicensee comes from one of:
/// - [`rap_to_klic`] applied to the operator-supplied RAP file
///   (license 1 / 2 NPDRM titles).
/// - `NP_KLIC_FREE` for free-license (license 3) NPDRM titles.
///   The convenience entry [`decrypt_self_to_elf_auto`] handles
///   this substitution.
///
/// Returns [`SceError::AesCbcDecryptFailed`] /
/// [`SceError::KeyEnvelopePadding`] if either layer key is wrong --
/// the envelope's zero-padding bytes self-certify a correct
/// decrypt.
pub fn decrypt_self_to_elf_npdrm(data: &[u8], klicensee: &[u8; 16]) -> Result<Vec<u8>, SceError> {
    let hdr = parse_sce_header(data)?;
    // The high bit of the SELF flags field marks an unencrypted
    // debug SELF; running the AES envelope decrypt over plaintext
    // would corrupt rather than peel. Reject explicitly at the
    // format level instead of relying on `decrypt_envelope`'s
    // `is_debug` skip (which is correct for the AES portion but
    // would still propagate non-envelope bytes downstream).
    if hdr.revision_flags & 0x8000 != 0 {
        return Err(SceError::DebugSelfUnsupported {
            revision_flags: hdr.revision_flags,
        });
    }
    let revision = hdr.revision_flags & 0x7FFF;
    let key =
        crate::crypto::npdrm_key_for_revision(revision).ok_or(SceError::NoAppKey { revision })?;
    let layer_key = klicensee_to_layer_key(klicensee);
    let envelope = decrypt_envelope(data, &hdr, &key.erk, &key.riv, Some(&layer_key))?;
    let sections = decrypt_sections_from_envelope(data, &hdr, &envelope)?;
    assemble_elf_from_sections(data, &sections)
}

/// Decrypt a SELF whose key class is not known up-front, dispatching
/// APP-keyed vs NPDRM via the presence of a type-3 supplemental
/// header.
///
/// `klicensee_lookup` is called with the title's `content_id` only
/// when the SELF is NPDRM-wrapped:
///
/// - `Some(klicensee)`: NPDRM path runs with the supplied bytes
///   (the caller is responsible for running [`rap_to_klic`] on the
///   RAP material first, or substituting `NP_KLIC_FREE` for free
///   licenses).
/// - `None`: errors with [`SceError::NoRapForNpdrmTitle`] naming the
///   `content_id` so the caller's diagnostic can name the expected
///   RAP path.
///
/// For non-NPDRM SELFs the closure is never invoked and the
/// APP-keyed path runs.
///
/// Free-license titles still flow through the closure: callers that
/// want the `NP_KLIC_FREE` default can return
/// `Some(cellgov_ps3_abi::sce::NP_KLIC_FREE)` for any content_id
/// matching the manifest's free-license declaration. RPCS3's
/// behaviour matches: license 3 substitutes `NP_KLIC_FREE` when no
/// RAP-derived klic is set in the KeyVault.
pub fn decrypt_self_to_elf_auto(
    data: &[u8],
    klicensee_lookup: impl FnOnce(&NpdHeaderInfo) -> Option<[u8; 16]>,
) -> Result<Vec<u8>, SceError> {
    match find_npd_header_info(data)? {
        None => crate::sce::decrypt_self_to_elf(data),
        Some(npd) => {
            let klicensee = resolve_npdrm_klicensee(&npd, klicensee_lookup)?;
            decrypt_self_to_elf_npdrm(data, &klicensee)
        }
    }
}

/// Resolve the klicensee bytes for an NPDRM SELF, given its NPD
/// header and the caller's RAP-keyed lookup.
///
/// Extracted from [`decrypt_self_to_elf_auto`] so the
/// license-dispatch decision is unit-testable in isolation: no
/// SELF blob required, no RAP file required, no AES round-trip.
/// Pure function of `(license, content_id)` and the lookup
/// closure's return value.
///
/// NPDRM declares three license types:
///
/// - License 1 (network) and 2 (local) both resolve the klic
///   from RAP material; they differ only in provenance, not in
///   this path. The lookup closure is the operator's RAP
///   resolver; `None` here means "no RAP available" and surfaces
///   as a typed error naming the title.
/// - License 3 (free) uses `NP_KLIC_FREE` unless a klic is
///   supplied via the lookup. A manifest-declared free-but-keyed
///   title can override the default by returning `Some(klic)`.
/// - Any other license value is structurally invalid; the SELF
///   does not declare a path the kernel would honour.
///
/// RPCS3's `DecryptNPDRM` is the cross-reference; line ranges
/// drift across RPCS3 commits, so this rustdoc deliberately
/// names the function only.
fn resolve_npdrm_klicensee(
    npd: &NpdHeaderInfo,
    klicensee_lookup: impl FnOnce(&NpdHeaderInfo) -> Option<[u8; 16]>,
) -> Result<[u8; 16], SceError> {
    match npd.license {
        1 | 2 => klicensee_lookup(npd).ok_or_else(|| SceError::NoRapForNpdrmTitle {
            content_id: npd.content_id.clone(),
        }),
        3 => Ok(klicensee_lookup(npd).unwrap_or(NP_KLIC_FREE)),
        other => Err(SceError::NpdrmBadLicense { got: other }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rap_to_klic_is_pure() {
        // Same RAP twice -> same klic. Pins the no-host-state
        // guarantee against accidental TLS/RNG use. Uses a
        // synthetic RAP so the test has no fixture dependency.
        let rap = [0x42u8; 16];
        let a = rap_to_klic(&rap);
        let b = rap_to_klic(&rap);
        assert_eq!(a, b);
    }

    // ----------------------------------------------------------------
    // resolve_npdrm_klicensee -- license dispatch unit tests.
    //
    // No SELF blob, no RAP file. Pure-function tests of the
    // license-branch decision; the NPDRM scheme defines three
    // license types (network=1, local=2, free=3) and these tests
    // pin our handling of each plus the invalid-license error
    // surface.
    // ----------------------------------------------------------------

    fn npd(license: u32, content_id: &str) -> NpdHeaderInfo {
        NpdHeaderInfo {
            license,
            content_id: content_id.to_string(),
        }
    }

    #[test]
    fn resolve_klicensee_license_network_with_rap_returns_klic() {
        let want = [0xABu8; 16];
        let got = resolve_npdrm_klicensee(&npd(1, "NPUA80001"), |_| Some(want)).unwrap();
        assert_eq!(got, want);
    }

    #[test]
    fn resolve_klicensee_license_local_with_rap_returns_klic() {
        let want = [0xCDu8; 16];
        let got = resolve_npdrm_klicensee(&npd(2, "NPUA80068"), |_| Some(want)).unwrap();
        assert_eq!(got, want);
    }

    #[test]
    fn resolve_klicensee_license_network_without_rap_errors_with_content_id() {
        // The typed error preserves the content_id so the boot-
        // path diagnostic can name which title needs a RAP rather
        // than collapsing to a generic decrypt failure.
        let err = resolve_npdrm_klicensee(&npd(1, "NPUA80001"), |_| None).unwrap_err();
        match err {
            SceError::NoRapForNpdrmTitle { content_id } => {
                assert_eq!(content_id, "NPUA80001");
            }
            other => panic!("expected NoRapForNpdrmTitle, got {other:?}"),
        }
    }

    #[test]
    fn resolve_klicensee_license_local_without_rap_errors_with_content_id() {
        // License 2 (local) is merged with license 1 (network)
        // in the dispatch; both resolve from RAP. See the rustdoc
        // on resolve_npdrm_klicensee for why.
        let err = resolve_npdrm_klicensee(&npd(2, "NPUA80068"), |_| None).unwrap_err();
        match err {
            SceError::NoRapForNpdrmTitle { content_id } => {
                assert_eq!(content_id, "NPUA80068");
            }
            other => panic!("expected NoRapForNpdrmTitle, got {other:?}"),
        }
    }

    #[test]
    fn resolve_klicensee_license_free_without_rap_returns_np_klic_free() {
        // License 3 falls back to NP_KLIC_FREE; this anchors the
        // default substitution so a future "license 3 must always
        // be lookup-supplied" refactor flips a named test.
        let got = resolve_npdrm_klicensee(&npd(3, "NPEA00000"), |_| None).unwrap();
        assert_eq!(got, NP_KLIC_FREE);
    }

    #[test]
    fn resolve_klicensee_license_free_with_rap_returns_supplied_klic() {
        // License 3 still honours an explicitly-supplied klic
        // (e.g. a manifest-declared free-but-keyed title); the
        // lookup wins over NP_KLIC_FREE. This anchors the override
        // semantics so a future "license 3 always uses
        // NP_KLIC_FREE" refactor flips a named test.
        let want = [0x77u8; 16];
        let got = resolve_npdrm_klicensee(&npd(3, "NPEA00000"), |_| Some(want)).unwrap();
        assert_eq!(got, want);
    }

    #[test]
    fn resolve_klicensee_license_zero_errors_npdrm_bad_license() {
        let err = resolve_npdrm_klicensee(&npd(0, "X"), |_| None).unwrap_err();
        assert!(matches!(err, SceError::NpdrmBadLicense { got: 0 }));
    }

    #[test]
    fn resolve_klicensee_license_four_errors_npdrm_bad_license() {
        let err = resolve_npdrm_klicensee(&npd(4, "X"), |_| None).unwrap_err();
        assert!(matches!(err, SceError::NpdrmBadLicense { got: 4 }));
    }

    #[test]
    fn resolve_klicensee_license_top_bit_set_errors_npdrm_bad_license() {
        // High-bit-set u32 value -- the kind of word a malformed
        // NPD header would deliver. Confirms the diagnostic prints
        // a large positive (unsigned) number rather than a
        // misleading negative one.
        let err = resolve_npdrm_klicensee(&npd(0x8000_0000, "X"), |_| None).unwrap_err();
        assert!(matches!(
            err,
            SceError::NpdrmBadLicense { got: 0x8000_0000 }
        ));
    }

    #[test]
    fn resolve_klicensee_license_u32_max_errors_npdrm_bad_license() {
        let err = resolve_npdrm_klicensee(&npd(u32::MAX, "X"), |_| None).unwrap_err();
        assert!(matches!(err, SceError::NpdrmBadLicense { got: u32::MAX }));
    }

    // ----------------------------------------------------------------
    // DebugSelfUnsupported guard -- pins the npdrm-entry rejection
    // independently of decrypt_envelope's `is_debug` skip.
    // Synthetic SCE header; no fixture dependency.
    // ----------------------------------------------------------------

    /// Build a minimal SCE container header (0x20 bytes) with the
    /// given `revision_flags`. Enough to satisfy `parse_sce_header`
    /// but not to advance into the AES path; the debug guard is
    /// the first thing to check after parse and fails fast.
    fn synthetic_sce_header_with_revision_flags(revision_flags: u16) -> Vec<u8> {
        let mut data = vec![0u8; 0x20];
        data[0..4].copy_from_slice(b"SCE\0");
        // header_version, category, metadata_offset, header_size,
        // encrypted_payload_size all left zero. `parse_sce_header`
        // only validates the magic; that's enough for the
        // debug-flag check to run.
        data[8..10].copy_from_slice(&revision_flags.to_be_bytes());
        data
    }

    #[test]
    fn decrypt_self_to_elf_npdrm_rejects_debug_self_with_high_bit_set() {
        let data = synthetic_sce_header_with_revision_flags(0x8000);
        let dummy_klic = [0u8; 16];
        let err = decrypt_self_to_elf_npdrm(&data, &dummy_klic).unwrap_err();
        match err {
            SceError::DebugSelfUnsupported { revision_flags } => {
                assert_eq!(revision_flags, 0x8000);
            }
            other => panic!("expected DebugSelfUnsupported, got {other:?}"),
        }
    }

    #[test]
    fn decrypt_self_to_elf_npdrm_rejects_debug_self_with_both_bits_set() {
        // High bit AND a non-zero revision in the low 15 bits.
        // Confirms the guard fires on the raw revision_flags value
        // (not the masked revision), and that the typed error
        // carries the full flags word.
        let data = synthetic_sce_header_with_revision_flags(0xC042);
        let dummy_klic = [0u8; 16];
        let err = decrypt_self_to_elf_npdrm(&data, &dummy_klic).unwrap_err();
        assert!(matches!(
            err,
            SceError::DebugSelfUnsupported {
                revision_flags: 0xC042
            }
        ));
    }

    #[test]
    fn decrypt_self_to_elf_npdrm_does_not_treat_high_revision_as_debug() {
        // revision_flags = 0x7FFF -- the highest non-debug
        // revision. Must NOT trigger DebugSelfUnsupported; should
        // fall through to the next failure (NoAppKey on the
        // missing 0x7FFF NPDRM key entry, which is fine -- the
        // guard isn't what's stopping us).
        let data = synthetic_sce_header_with_revision_flags(0x7FFF);
        let dummy_klic = [0u8; 16];
        let err = decrypt_self_to_elf_npdrm(&data, &dummy_klic).unwrap_err();
        assert!(!matches!(err, SceError::DebugSelfUnsupported { .. }));
    }
}

// ----------------------------------------------------------------
// Oracle-vector tests (RAP + EBOOT fixture-dependent).
//
// Gated behind the `npdrm-oracle-vectors` feature so the default
// gate compiles RAP-free. The constants and the include_bytes!
// helpers live entirely inside the gate; without the feature
// the crate does not look for the operator-supplied RAP or
// EBOOT files. CI runs one job with the feature on (on a host
// carrying the fixtures) to exercise the localization vectors
// plus the end-to-end ELF self-certification tests.
//
// Correctness anchor: the end-to-end `*_eboot_decrypts_to_parseable_elf`
// tests. The format self-certifies a correct decrypt (envelope
// padding zeros, ELF magic). The smaller klic vectors are
// localization aids -- if the end-to-end test breaks, the klic
// vectors tell you whether the regression is in `rap_to_klic`
// or downstream.
// ----------------------------------------------------------------

#[cfg(all(test, feature = "npdrm-oracle-vectors"))]
mod oracle_vectors {
    use super::*;

    /// flOw (NPUA80001) RAP, included from the operator-supplied
    /// VFS tree.
    const FLOW_RAP: [u8; 16] = include_bytes_array_flow_rap();
    /// flOw klic, computed once via `oracle/rap_to_klic_oracle.py`
    /// against `FLOW_RAP` and frozen here.
    ///
    /// Status: **localization aid**, not a correctness anchor.
    /// The pair `(FLOW_RAP, FLOW_EXPECTED_KLIC)` is a witness of
    /// `(real RAP, fixed reverse-engineered algorithm)`. If the
    /// end-to-end `flow_eboot_decrypts_to_parseable_elf` test
    /// breaks, this vector tells you whether the regression is in
    /// `rap_to_klic` (this test flips too) or downstream
    /// (envelope / reassembly; this test still passes).
    const FLOW_EXPECTED_KLIC: [u8; 16] = [
        0x04, 0x43, 0xFA, 0x57, 0x9C, 0xB8, 0xEF, 0xBF, 0xE5, 0xA8, 0x98, 0xAE, 0xF2, 0x81, 0x8E,
        0xC1,
    ];

    /// SSHD (NPUA80068) RAP. Second independent input alongside
    /// flOw: structural breaks flip both, input-dependent breaks
    /// flip only one.
    const SSHD_RAP: [u8; 16] = include_bytes_array_sshd_rap();
    const SSHD_EXPECTED_KLIC: [u8; 16] = [
        0xDA, 0x60, 0x18, 0x39, 0xD4, 0x18, 0xCF, 0x8C, 0x91, 0xEC, 0xDE, 0x76, 0x92, 0xED, 0xCB,
        0x47,
    ];

    /// flOw EBOOT.BIN -- the NPDRM-wrapped SELF. ~9.7 MiB; pulled
    /// in by `include_bytes!` so the test binary self-contains
    /// the inputs and decryption runs without runtime fs I/O.
    const FLOW_EBOOT: &[u8] =
        include_bytes!("../../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN");
    /// SSHD EBOOT.BIN.
    const SSHD_EBOOT: &[u8] =
        include_bytes!("../../../tools/rpcs3/dev_hdd0/game/NPUA80068/USRDIR/EBOOT.BIN");

    const fn include_bytes_array_flow_rap() -> [u8; 16] {
        // include_bytes!() returns &'static [u8; N]; deref once
        // and copy into a fixed array so it can live in a `const`.
        *include_bytes!(
            "../../../tools/rpcs3/dev_hdd0/home/00000001/exdata/UP9000-NPUA80001_00-FLOWPS3PROMOTION.rap"
        )
    }

    const fn include_bytes_array_sshd_rap() -> [u8; 16] {
        *include_bytes!(
            "../../../tools/rpcs3/dev_hdd0/home/00000001/exdata/UP9000-NPUA80068_00-STARDUSTFULL0001.rap"
        )
    }

    /// Validate that the decrypted bytes look like a 64-bit
    /// big-endian PS3 ELF. Format-level sanity, no cross-tool
    /// comparison. ELF magic + EI_CLASS=ELFCLASS64 + EI_DATA=
    /// ELFDATA2MSB + e_machine=EM_PPC64 + the declared phdr table
    /// fits inside the buffer.
    ///
    /// The phdr-fits check (`e_phoff + e_phnum * e_phentsize <=
    /// elf.len()`) is the load-bearing one: a subtly-wrong
    /// decrypt that still produces valid ELF magic is the most
    /// plausible regression shape, and "magic + e_phnum > 0"
    /// alone passes such cases. "Declared phdrs are actually
    /// present" is the real format tripwire.
    fn assert_is_ps3_ppc64_elf(elf: &[u8]) {
        assert!(elf.len() >= 0x40, "ELF shorter than ehdr");
        assert_eq!(&elf[..4], b"\x7fELF", "ELF magic");
        assert_eq!(elf[4], 2, "EI_CLASS must be ELFCLASS64");
        assert_eq!(elf[5], 2, "EI_DATA must be ELFDATA2MSB (big-endian)");
        let e_machine = u16::from_be_bytes([elf[0x12], elf[0x13]]);
        assert_eq!(e_machine, 21, "e_machine must be EM_PPC64 (21)");
        let e_phoff = u64::from_be_bytes(
            elf[0x20..0x28]
                .try_into()
                .expect("invariant: 8-byte slice converts to [u8; 8]"),
        ) as usize;
        let e_phentsize = u16::from_be_bytes([elf[0x36], elf[0x37]]) as usize;
        let e_phnum = u16::from_be_bytes([elf[0x38], elf[0x39]]) as usize;
        assert!(e_phnum > 0, "ELF must have at least one program header");
        let phdr_table_end = e_phoff
            .checked_add(
                e_phnum
                    .checked_mul(e_phentsize)
                    .expect("phdr size overflow"),
            )
            .expect("phdr offset overflow");
        assert!(
            phdr_table_end <= elf.len(),
            "phdr table extends past ELF buffer: e_phoff=0x{e_phoff:x} + \
             e_phnum={e_phnum} * e_phentsize={e_phentsize} = {phdr_table_end} \
             > {} (ELF length)",
            elf.len(),
        );
    }

    #[test]
    fn rap_to_klic_matches_witness_flow() {
        let got = rap_to_klic(&FLOW_RAP);
        assert_eq!(
            got, FLOW_EXPECTED_KLIC,
            "flOw klic drift: rap_to_klic produced bytes that \
             disagree with the witness. Localization: regression is \
             in rap_to_klic (the end-to-end test will also flip)."
        );
    }

    #[test]
    fn rap_to_klic_matches_witness_sshd() {
        let got = rap_to_klic(&SSHD_RAP);
        assert_eq!(
            got, SSHD_EXPECTED_KLIC,
            "SSHD klic drift: two independent RAPs validate the \
             algorithm. If only one flips the bug is input-dependent \
             (likely the 5-round dance); if both flip the bug is \
             structural."
        );
    }

    #[test]
    fn flow_eboot_decrypts_to_parseable_elf() {
        // Correctness anchor for the whole NPDRM pipeline. The
        // format self-certifies: wrong klic -> envelope padding
        // nonzero -> KeyEnvelopePadding; wrong section keys ->
        // malformed metadata directory -> typed SceError;
        // correct decrypt -> valid PS3 ELF. No cross-tool oracle
        // required.
        let klic = rap_to_klic(&FLOW_RAP);
        let elf = decrypt_self_to_elf_npdrm(FLOW_EBOOT, &klic)
            .expect("flOw NPDRM decrypt: padding + section hashes self-certify");
        assert_is_ps3_ppc64_elf(&elf);
    }

    #[test]
    fn sshd_eboot_decrypts_to_parseable_elf() {
        let klic = rap_to_klic(&SSHD_RAP);
        let elf = decrypt_self_to_elf_npdrm(SSHD_EBOOT, &klic)
            .expect("SSHD NPDRM decrypt: padding + section hashes self-certify");
        assert_is_ps3_ppc64_elf(&elf);
    }
}
