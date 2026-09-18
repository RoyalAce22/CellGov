//! The base installers: turn a retail PKG or disc image into a
//! `dev_hdd0`/`dev_bdvd` tree, an installed RAP, and a
//! [`crate::store`] record naming all three.
//!
//! The pure container work lives in [`crate::pkg`] / [`crate::iso`];
//! the filesystem shell is [`super::staging`].
//!
//! Both installers run the decrypt-proof, so they exist only with the
//! `decrypt` feature; the record types, the store layout, and the
//! uninstall side stay available in every build.

#![cfg_attr(
    not(feature = "decrypt"),
    allow(
        dead_code,
        unused_imports,
        reason = "the RAP planner is reachable only from the gated installers; the feature-on build lints them"
    )
)]

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::firmware_install::{self, FirmwareInstallOutcome};
use crate::game_install::error::GameInstallError;
use crate::game_install::staging::{
    build_record, commit, dir_non_empty, emit_totals, parse_identity, prepare_staging,
    run_or_clean, sha256_of, stage_tree, validate_content_id, write_and_sync, InstallOptions,
    SfoIdentity, StagedData, StagedFile, StagedRap,
};
use crate::iso;
use crate::keys::KeyVault;
use crate::npdrm::{self, NpdHeaderInfo, NpdLicense};
use crate::param_sfo;
use crate::pkg::{self, PkgEntryKind};
use crate::progress::{Phase, ProgressSink};
use crate::pup;
use crate::sce;
use crate::self_image::is_sce_wrapped;
use crate::store::layout::{staging_sibling, Artifact, StoreLayout, TitleId};
use crate::store::lock::lock_artifact;
use crate::store::record::{ArtifactRecord, RapRecord, TitleRecord};
use cellgov_ps3_abi::format::elf::ELF_MAGIC;
use cellgov_ps3_abi::format::title_tree::DISC_UPDATE_PUP;

/// PARAM.SFO categories that mark a disc title (`DG` disc game,
/// `GD` disc game/data).
const DISC_CATEGORIES: [&str; 2] = ["DG", "GD"];

/// What a completed install produced, for the CLI to report.
#[derive(Debug, Clone)]
pub struct GameInstallOutcome {
    /// PARAM.SFO `TITLE_ID` (the game-directory key).
    pub title_id: String,
    /// Full NPD content id (the RAP-filename key), or the title-id for
    /// an APP-keyed title with no NPD header.
    pub content_id: String,
    /// The committed game-tree directory.
    pub game_dir: PathBuf,
    /// Whether a RAP was installed into `exdata/`.
    pub rap_installed: bool,
    /// Whether the install dropped a supplied RAP the title's license
    /// does not consume.
    pub rap_ignored: bool,
    /// Number of distinct files the committed tree holds, equal to the
    /// record's file count. Two container entries whose paths normalize
    /// to the same key are one file on disk and one record key, so the
    /// staged-entry count would over-report both.
    pub file_count: usize,
    /// The written install record.
    pub record_path: PathBuf,
    /// Refusals the RAP and tree renames outwaited, summed. See
    /// [`rename_with_retry`](crate::store::rename_with_retry).
    pub rename_retries: u32,
    /// The system software the disc shipped, as the install settled it.
    ///
    /// `None` for:
    ///
    /// - a PKG;
    /// - a disc with no update package;
    /// - an install that declined to open it
    ///   ([`InstallOptions::shipped_firmware`]).
    pub shipped_firmware: Option<ShippedFirmware>,
}

/// The system software a disc image ships in `PS3_UPDATE/`, registered
/// as a store firmware entry and recorded on the title.
#[derive(Debug, Clone)]
pub struct ShippedFirmware {
    /// Version key of the package, and of the `firmware/<key>` entry it
    /// names; what the title record carries as `shipped_firmware`.
    pub version: String,
    /// Whether this install created the entry.
    pub disposition: ShippedFirmwareDisposition,
}

/// Whether this install created the disc's firmware entry, or found one
/// recorded.
#[derive(Debug, Clone)]
pub enum ShippedFirmwareDisposition {
    /// This install unpacked the disc's package into a new entry.
    Installed(Box<FirmwareInstallOutcome>),
    /// The store already recorded an entry under that version, so this
    /// install unpacked nothing.
    AlreadyInstalled {
        /// Whether the recorded entry came from a package with the same
        /// bytes as the disc's.
        same_pup: bool,
    },
}

/// Forwards a nested install's item names and drops everything else.
///
/// The firmware installer under a disc install reports to the disc
/// install's bar, whose phase, totals and file counter describe the
/// title. The package names pass through, so the indeterminate firmware
/// phase shows the current package.
struct ItemsOnly<'a>(&'a dyn ProgressSink);

impl ProgressSink for ItemsOnly<'_> {
    fn phase(&self, _code: u8) {}
    fn totals(&self, _items: usize, _amount: u64) {}
    fn preset_done(&self, _amount: u64) {}
    fn item_started(&self, name: &str) {
        self.0.item_started(name);
    }
    fn advanced(&self, _delta: u64) {}
    fn item_finished(&self) {}
    fn finished(&self) {}
}

/// The bytes of one disc file: borrowed from the image when the file is
/// one extent, copied when it is several.
fn entry_bytes<'i>(entry: &iso::IsoEntry, image: &'i [u8]) -> Result<Cow<'i, [u8]>, iso::IsoError> {
    let slices = entry.extent_slices(image)?;
    Ok(match slices.as_slice() {
        [one] => Cow::Borrowed(one),
        many => Cow::Owned(many.concat()),
    })
}

/// Settle the system software the disc ships, before the caller stages
/// any of the title.
///
/// This function validates the package and reads its version in place.
/// It does not unpack a version the store already records. An
/// unrecorded version goes through the firmware installer under its own
/// entry and lock, with `force` off, so the installer reports an
/// unrecorded tree under the entry directory and never overwrites it.
///
/// Returns `None` when:
///
/// - the disc carries no package;
/// - `opts` declines to open it.
///
/// # Errors
///
/// [`GameInstallError::ShippedFirmware`] for every refusal of the
/// package or the entry, and
/// [`GameInstallError::ShippedFirmwareVersionMismatch`] when the
/// package's version entry and its unpacked tree disagree.
#[cfg(feature = "decrypt")]
fn shipped_firmware(
    entries: &[iso::IsoEntry],
    image: &[u8],
    keys: &KeyVault,
    output_dir: &Path,
    opts: InstallOptions<'_>,
) -> Result<Option<ShippedFirmware>, GameInstallError> {
    if !opts.shipped_firmware {
        return Ok(None);
    }
    let Some(entry) = entries.iter().find(|e| e.path == DISC_UPDATE_PUP) else {
        return Ok(None);
    };
    // A directory under the package's name is not an absent package:
    // the image holds something there, and it is not a PUP.
    if entry.kind != iso::IsoEntryKind::File {
        return Err(GameInstallError::ShippedFirmwareNotAFile);
    }
    let pup_data = entry_bytes(entry, image)?;
    let refused = |source: firmware_install::FirmwareInstallError| {
        GameInstallError::ShippedFirmware { source }
    };
    let parsed = pup::parse(&pup_data).map_err(|e| refused(e.into()))?;
    pup::validate_hashes(&pup_data, &parsed, keys).map_err(|e| refused(e.into()))?;
    let version = pup::version_key(&pup_data, &parsed).map_err(|e| refused(e.into()))?;

    if let Some(existing) =
        firmware_install::installed_record(output_dir, &version).map_err(refused)?
    {
        let same_pup = existing.source.sha256 == sha256_of(&pup_data);
        return Ok(Some(ShippedFirmware {
            version,
            disposition: ShippedFirmwareDisposition::AlreadyInstalled { same_pup },
        }));
    }

    let progress = opts.progress;
    progress.phase(Phase::InstallingFirmware.code());
    let installed =
        firmware_install::install_pup(&pup_data, keys, output_dir, false, &ItemsOnly(progress));
    // The last package name would otherwise stand until the first
    // staged file replaces it.
    progress.item_started("");
    let disposition = match installed {
        Ok(outcome) if outcome.version != version => {
            return Err(GameInstallError::ShippedFirmwareVersionMismatch {
                declared: version,
                extracted: outcome.version,
            });
        }
        Ok(outcome) => ShippedFirmwareDisposition::Installed(Box::new(outcome)),
        Err(refusal) => settle_installer_refusal(&version, refusal)?,
    };
    Ok(Some(ShippedFirmware {
        version,
        disposition,
    }))
}

/// What a firmware installer refusal means for the disc install.
///
/// The installer learns the tree's version only after it unpacks, so
/// its duplicate-version refusals name a version the pre-check did not
/// see recorded. Two cases:
///
/// - The version landed between the two reads. Another writer installed
///   it, and the disc reuses that entry.
/// - The package's `version.txt` entry does not spell the tree's
///   version. The package is at odds with its own tree, and the
///   installer committed that tree under the tree's version.
///
/// Every other refusal stands.
fn settle_installer_refusal(
    declared: &str,
    refusal: firmware_install::FirmwareInstallError,
) -> Result<ShippedFirmwareDisposition, GameInstallError> {
    use firmware_install::FirmwareInstallError as Fw;
    let (version, same_pup) = match refusal {
        Fw::VersionInstalled { version, .. } => (version, true),
        Fw::VersionInstalledFromAnotherPup { version, .. } => (version, false),
        other => return Err(GameInstallError::ShippedFirmware { source: other }),
    };
    if version == declared {
        Ok(ShippedFirmwareDisposition::AlreadyInstalled { same_pup })
    } else {
        Err(GameInstallError::ShippedFirmwareVersionMismatch {
            declared: declared.to_string(),
            extracted: version,
        })
    }
}

/// Whether a license consumes a RAP. Only network/local do; free and
/// APP-keyed (no NPD header) titles resolve their klicensee without one
/// (free falls back to the vault's free klicensee).
fn rap_consumed(license: Option<NpdLicense>) -> bool {
    match license {
        Some(NpdLicense::Network) | Some(NpdLicense::Local) => true,
        Some(NpdLicense::Free) | None => false,
    }
}

/// The RAP to stage for a title, or `None` for disc/free/no-RAP titles.
///
/// A RAP supplied for a title [`rap_consumed`] rejects is dropped here
/// and never staged, committed, or recorded. The caller reports the
/// drop through [`GameInstallOutcome::rap_ignored`].
fn plan_staged_rap(
    rap_needed: bool,
    rap: Option<&[u8]>,
    content_id: &str,
    staging_root: &Path,
    exdata: &Path,
) -> Option<StagedRap> {
    if !rap_needed {
        return None;
    }
    let r = rap.expect("invariant: rap_needed without a RAP returns RapRequired above");
    let filename = format!("{content_id}.rap");
    Some(StagedRap {
        staged_path: staging_root.join("rap").join(&filename),
        final_path: exdata.join(&filename),
        record: RapRecord {
            filename,
            sha256: sha256_of(r),
        },
    })
}

/// Install a retail PKG (PSN/retail HDD title) into `output_dir`'s
/// `dev_hdd0` tree, installing the RAP and writing the title-base
/// record for that root.
///
/// # Errors
///
/// [`GameInstallError::PreStore`] when the root still holds the
/// pre-store layout. Nothing is read or staged before this refusal.
///
/// [`GameInstallError::Locked`] when another process holds this title's
/// base. The install claims the base once the identity gates pass and
/// before the staging sweep, so this refusal also stages nothing.
///
/// The shared identity / RAP / staging / record failures also apply.
#[cfg(feature = "decrypt")]
pub fn install_pkg(
    pkg_bytes: &[u8],
    rap: Option<&[u8]>,
    keys: &KeyVault,
    output_dir: &Path,
    opts: InstallOptions<'_>,
) -> Result<GameInstallOutcome, GameInstallError> {
    crate::store::pre_store::preflight(output_dir)?;
    let progress = opts.progress;
    progress.phase(Phase::Reading.code());
    let archive = pkg::extract(pkg_bytes, keys)?;

    // PARAM.SFO -> identity + HDD-game gate.
    let sfo_file = archive
        .files
        .iter()
        .find(|f| f.name == "PARAM.SFO")
        .ok_or(GameInstallError::NoParamSfo)?;
    let SfoIdentity {
        title_id,
        category,
        title,
        version,
        system_ver,
    } = parse_identity(archive.file_data(sfo_file))?;
    if category != "HG" {
        return Err(GameInstallError::NotHddGame { category });
    }
    // The header's content-id field embeds the 9-char title-id (e.g.
    // `UP9000-NPUA80001_00-...` carries `NPUA80001`), so a containment
    // check is robust to the prefix/suffix the SFO TITLE_ID omits.
    if !archive.header.content_id.is_empty() && !archive.header.content_id.contains(&title_id) {
        return Err(GameInstallError::TitleIdMismatch {
            header: archive.header.content_id.clone(),
            sfo: title_id,
        });
    }

    // EBOOT -> license type + full content id.
    let eboot = archive
        .files
        .iter()
        .find(|f| f.name == "USRDIR/EBOOT.BIN")
        .ok_or(GameInstallError::NoEboot)?;
    let eboot_data = archive.file_data(eboot);
    let npd = npdrm::find_npd_header_info(eboot_data).map_err(GameInstallError::Npd)?;
    // The EBOOT's NPD content-id must also embed the title-id, tying
    // the executable's own identity to the PARAM.SFO / header.
    if let Some(n) = &npd {
        if !n.content_id.contains(&title_id) {
            return Err(GameInstallError::ContentIdMismatch {
                npd: n.content_id.clone(),
                title_id,
            });
        }
    }
    let content_id = npd
        .as_ref()
        .map(|n| n.content_id.clone())
        .unwrap_or_else(|| title_id.clone());

    // Both keys become path components (game/<title-id>,
    // exdata/<content-id>.rap); reject anything unsafe before use.
    validate_content_id(&title_id)?;
    validate_content_id(&content_id)?;

    // Layout. The pre-commit batch lives under one staging root:
    // `tree/` is the game tree, `rap/` holds the staged RAP.
    let layout = StoreLayout::new(output_dir);
    let exdata = layout.live_exdata_dir();
    let final_dir = output_dir.join("dev_hdd0").join("game").join(&title_id);
    let staging_root = staging_sibling(&final_dir)?;
    let tree_staging = staging_root.join("tree");
    // Resolved before staging: only `run_or_clean` discards the staging
    // root, so a fallible step between it and `commit` would leave the
    // staged tree behind.
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new(&title_id)?,
    };
    let store_path = layout.store_path_of(&final_dir)?;
    // The install holds this claim to the end. The target gate, the
    // staging sweep, and both commit renames touch paths that a second
    // install or an uninstall of this title also touches.
    let _lock = lock_artifact(&layout, &artifact)?;

    if dir_non_empty(&final_dir)? && !opts.force {
        return Err(GameInstallError::TargetExists { path: final_dir });
    }

    // Fail fast on the RAP contract before staging anything.
    let license = npd.as_ref().map(|n| n.license);
    let rap_needed = rap_consumed(license);
    if rap_needed && rap.is_none() {
        return Err(GameInstallError::RapRequired { content_id });
    }
    if let Some(r) = rap {
        if r.len() != 16 {
            return Err(GameInstallError::RapWrongSize { len: r.len() });
        }
    }

    let staged: Vec<StagedFile> = archive
        .files
        .iter()
        .map(|f| StagedFile {
            path: f.name.clone(),
            is_dir: f.kind == PkgEntryKind::Directory,
            data: StagedData::Bytes(archive.file_data(f)),
        })
        .collect();
    emit_totals(progress, &staged);

    // Build the whole batch (tree + RAP + proof) under the staging root.
    prepare_staging(&staging_root, progress)?;
    let (file_digests, staged_rap) = run_or_clean(&staging_root, || {
        progress.phase(Phase::Staging.code());
        let file_digests = stage_tree(&staged, &tree_staging, progress)?;
        let staged_rap = plan_staged_rap(rap_needed, rap, &content_id, &staging_root, &exdata);
        if let Some(sr) = &staged_rap {
            let rap_bytes = rap.expect(
                "invariant: plan_staged_rap returns Some only when rap_needed, so rap is set",
            );
            write_and_sync(&sr.staged_path, rap_bytes)?;
        }
        // The proof resolves the RAP the way the load path will -- a
        // content-id-keyed read -- but against the staged copy, so a
        // fault before commit touches no live exdata.
        let rap_staging = staging_root.join("rap");
        let resolver = move |n: &NpdHeaderInfo| -> Option<npdrm::Rap> {
            let rap_path = rap_staging.join(format!("{}.rap", n.content_id));
            let bytes = std::fs::read(&rap_path).ok()?;
            let arr: [u8; 16] = bytes.as_slice().try_into().ok()?;
            Some(npdrm::Rap(arr))
        };
        progress.phase(Phase::Proving.code());
        npdrm::decrypt_self_to_elf_auto(eboot_data, keys, resolver)
            .map_err(GameInstallError::DecryptProof)?;
        Ok((file_digests, staged_rap))
    })?;

    let rap_installed = staged_rap.is_some();
    let rap_ignored = rap.is_some() && !rap_installed;
    // The source hash is a full read of the container: its own phase,
    // or a multi-gigabyte container's hash time hides under the proof
    // label.
    progress.phase(Phase::Hashing.code());
    let record = build_record(
        "pkg",
        pkg_bytes,
        ArtifactRecord {
            kind: artifact.kind(),
            version,
            store_path,
        },
        file_digests,
        TitleRecord {
            title_id: title_id.clone(),
            content_id: content_id.clone(),
            category,
            title,
            distribution: "psn-hdd".to_string(),
            system_ver,
            shipped_firmware: None,
        },
        staged_rap.as_ref().map(|r| r.record.clone()),
    );
    let committed = commit(
        &staging_root,
        &tree_staging,
        &final_dir,
        staged_rap.as_ref(),
        &layout.record_path(&artifact),
        &record,
        progress,
    )?;

    progress.finished();
    Ok(GameInstallOutcome {
        title_id,
        content_id,
        game_dir: final_dir,
        rap_installed,
        rap_ignored,
        file_count: record.files.len(),
        record_path: committed.record_path,
        rename_retries: committed.rename_retries,
        shipped_firmware: None,
    })
}

/// Install a decrypted disc image's `PS3_GAME/` tree (an APP-keyed
/// disc title) into `output_dir`'s `dev_bdvd/<title-id>/` tree.
///
/// The record's source hash is over `image` itself, so the image must
/// already be plaintext. There is no RAP -- disc EBOOTs are APP-keyed
/// -- so the decrypt-proof runs through `sce::decrypt_self_to_elf`
/// under `keys`.
///
/// The install registers the disc's system software in `PS3_UPDATE/` as
/// an ordinary `firmware/<version>` entry before it stages the title. It
/// validates the package and reads its version in place. It unpacks the
/// package only when the store records no entry under that version. The
/// title record carries the version as `shipped_firmware`. Every disc
/// that ships the version shares the entry. A later fault in this
/// install leaves it in place, and a title uninstall never removes it.
///
/// # Errors
///
/// [`GameInstallError::PreStore`] when the root still holds the
/// pre-store layout. Nothing is read or staged before this refusal.
///
/// [`GameInstallError::DiscImageEncrypted`] for an image still carrying
/// its disc encryption, refused before anything is staged.
///
/// [`GameInstallError::ShippedFirmware`] and
/// [`GameInstallError::ShippedFirmwareVersionMismatch`] when the disc's
/// package cannot be registered; nothing of the title is staged.
///
/// [`GameInstallError::Locked`] when another process holds this title's
/// base. A disc base and an HDD base of one title share one record, so
/// they share one claim. An [`install_pkg`] of the same title raises
/// this refusal too.
#[cfg(feature = "decrypt")]
pub fn install_iso(
    image: &[u8],
    keys: &KeyVault,
    output_dir: &Path,
    opts: InstallOptions<'_>,
) -> Result<GameInstallOutcome, GameInstallError> {
    crate::store::pre_store::preflight(output_dir)?;
    let progress = opts.progress;
    progress.phase(Phase::Reading.code());
    let entries = iso::read_iso(image)?;

    let sfo_entry = entries
        .iter()
        .find(|e| e.path == "PS3_GAME/PARAM.SFO")
        .ok_or(GameInstallError::NoDiscParamSfo)?;
    let sfo_bytes = sfo_entry.read_data(image)?;
    let SfoIdentity {
        title_id,
        category,
        title,
        version,
        system_ver,
    } = match parse_identity(&sfo_bytes) {
        Err(GameInstallError::Sfo(param_sfo::SfoError::BadMagic(head))) => {
            return Err(GameInstallError::DiscImageEncrypted {
                path: "PS3_GAME/PARAM.SFO",
                head,
            });
        }
        other => other?,
    };
    if !DISC_CATEGORIES.contains(&category.as_str()) {
        return Err(GameInstallError::NotDiscGame { category });
    }
    // The title-id becomes the dev_bdvd/<title-id> directory key.
    validate_content_id(&title_id)?;

    let eboot_bytes = entries
        .iter()
        .find(|e| e.path == "PS3_GAME/USRDIR/EBOOT.BIN")
        .ok_or(GameInstallError::NoDiscEboot)?
        .read_data(image)?;
    // Metadata files can sit in a plaintext region of an otherwise
    // encrypted disc. ISO 9660 / ECMA-119 describes the directory
    // structure only and says nothing about a file's contents. The
    // EBOOT's own first bytes therefore settle whether the image is
    // decrypted, before the tree streams to disk. Disc encryption is
    // an unpadded per-sector block cipher and keeps every file's
    // length, so an EBOOT too short to hold a magic reaches the proof,
    // which refuses it by length.
    if let Some(&head) = eboot_bytes.first_chunk::<4>() {
        if !is_sce_wrapped(&eboot_bytes) && head != ELF_MAGIC {
            return Err(GameInstallError::DiscImageEncrypted {
                path: "PS3_GAME/USRDIR/EBOOT.BIN",
                head,
            });
        }
    }

    let layout = StoreLayout::new(output_dir);
    let final_dir = output_dir.join("dev_bdvd").join(&title_id);
    let staging_dir = staging_sibling(&final_dir)?;
    // Resolved before staging, as in `install_pkg`.
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new(&title_id)?,
    };
    let store_path = layout.store_path_of(&final_dir)?;
    let _lock = lock_artifact(&layout, &artifact)?;

    if dir_non_empty(&final_dir)? && !opts.force {
        return Err(GameInstallError::TargetExists { path: final_dir });
    }

    // The disc's own system software goes first: a refusal there stages
    // none of the title, and a title fault after it leaves an ordinary
    // firmware entry behind rather than half of one.
    let shipped = shipped_firmware(&entries, image, keys, output_dir, opts)?;

    // Each file stays in the image until stage_tree streams it to disk,
    // so a full BD image installs in bounded memory.
    let staged: Vec<StagedFile> = entries
        .iter()
        .map(|e| {
            Ok(StagedFile {
                path: e.path.clone(),
                is_dir: e.kind == iso::IsoEntryKind::Directory,
                data: StagedData::Slices(e.extent_slices(image)?),
            })
        })
        .collect::<Result<_, iso::IsoError>>()?;
    emit_totals(progress, &staged);

    prepare_staging(&staging_dir, progress)?;
    let file_digests = run_or_clean(&staging_dir, || {
        progress.phase(Phase::Staging.code());
        let file_digests = stage_tree(&staged, &staging_dir, progress)?;
        // APP-keyed disc EBOOT: prove it decrypts end-to-end, discard.
        progress.phase(Phase::Proving.code());
        sce::decrypt_self_to_elf(&eboot_bytes, keys).map_err(GameInstallError::DecryptProof)?;
        Ok(file_digests)
    })?;

    progress.phase(Phase::Hashing.code());
    let record = build_record(
        "iso",
        image,
        ArtifactRecord {
            kind: artifact.kind(),
            version,
            store_path,
        },
        file_digests,
        TitleRecord {
            title_id: title_id.clone(),
            content_id: title_id.clone(),
            category,
            title,
            distribution: "disc-iso".to_string(),
            system_ver,
            shipped_firmware: shipped.as_ref().map(|s| s.version.clone()),
        },
        None,
    );
    // Disc: the staging root is itself the tree (no `tree/` nesting),
    // and there is no RAP.
    let committed = commit(
        &staging_dir,
        &staging_dir,
        &final_dir,
        None,
        &layout.record_path(&artifact),
        &record,
        progress,
    )?;

    progress.finished();
    Ok(GameInstallOutcome {
        title_id: title_id.clone(),
        content_id: title_id,
        game_dir: final_dir,
        rap_installed: false,
        rap_ignored: false,
        file_count: record.files.len(),
        record_path: committed.record_path,
        rename_retries: committed.rename_retries,
        shipped_firmware: shipped,
    })
}

#[cfg(test)]
#[path = "tests/base_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/disc_tests.rs"]
mod disc_tests;

#[cfg(test)]
#[path = "tests/shipped_firmware_tests.rs"]
mod shipped_firmware_tests;
