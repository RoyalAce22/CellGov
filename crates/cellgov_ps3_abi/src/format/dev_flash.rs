//! Retail firmware (`dev_flash`) content facts.

/// Names of the three flash mounts, flash 1 first.
///
/// These are bare directory names, the spelling the host store lays
/// out. The guest addresses flash 1 as [`GUEST_FLASH_MOUNT`].
pub const FLASH_MOUNTS: [&str; 3] = ["dev_flash", "dev_flash2", "dev_flash3"];

/// Flash 1, the mount that holds the firmware image, as a bare
/// directory name.
pub const FLASH_MOUNT: &str = FLASH_MOUNTS[0];

/// The flash mounts that sit beside [`FLASH_MOUNT`].
pub const SIBLING_FLASH_MOUNTS: [&str; 2] = [FLASH_MOUNTS[1], FLASH_MOUNTS[2]];

// A flash mount missing from SIBLING_FLASH_MOUNTS lands inside flash 1:
// prefix routing files every name without a sibling prefix under
// FLASH_MOUNT.
const _: () = assert!(SIBLING_FLASH_MOUNTS.len() + 1 == FLASH_MOUNTS.len());

/// The path the guest addresses flash 1 at.
pub const GUEST_FLASH_MOUNT: &str = "/dev_flash";

/// Path components of the version file, relative to the `dev_flash`
/// mount root.
///
/// Every retail firmware image carries it, and it holds the version a
/// user sees.
pub const VERSION_TXT_COMPONENTS: [&str; 3] = ["vsh", "etc", "version.txt"];

/// The system shell's directory, relative to the `dev_flash` mount
/// root, `/`-separated.
///
/// Retail firmware puts the shell and the modules it loads by path
/// here, at the same place in every revision.
pub const VSH_MODULE_DIR: &str = "vsh/module";

/// The system shell's executable, inside [`VSH_MODULE_DIR`].
pub const VSH_SELF: &str = "vsh.self";

/// The record the version file opens with.
pub const VERSION_TXT_RELEASE_FIELD: &str = "release";

/// Major digits in the `release` record's fixed-width version field.
pub const VERSION_TXT_MAJOR_DIGITS: usize = 2;

/// Minor digits in that field. The leading two are the version a user
/// sees; the rest are a sub-revision.
pub const VERSION_TXT_MINOR_DIGITS: usize = 4;

/// Minor digits kept in the version a user sees, the leading part of
/// [`VERSION_TXT_MINOR_DIGITS`].
///
/// The retail 1.02 image writes its field at this width, with no
/// sub-revision digits: `release:01.02:`.
pub const VERSION_TXT_MINOR_DIGITS_SHOWN: usize = 2;

/// The version a user sees, from a `vsh/etc/version.txt` text:
/// `release:04.9100:` reads as `4.91`.
///
/// The file opens with a `release:<version>:` record whose version is
/// zero-padded: [`VERSION_TXT_MAJOR_DIGITS`] major digits, a dot, then
/// either [`VERSION_TXT_MINOR_DIGITS`] minor digits or only the
/// [`VERSION_TXT_MINOR_DIGITS_SHOWN`] a user sees. `None` unless the
/// leading [`VERSION_TXT_RELEASE_FIELD`] record carries one of those
/// two shapes.
pub fn parse_version_txt(text: &str) -> Option<String> {
    let (record, rest) = text.split_once(':')?;
    if record != VERSION_TXT_RELEASE_FIELD {
        return None;
    }
    let field = rest.get(..rest.find(':')?)?;

    let (major, minor) = field.split_once('.')?;
    let fixed_width = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    if !fixed_width(major, VERSION_TXT_MAJOR_DIGITS)
        || !(fixed_width(minor, VERSION_TXT_MINOR_DIGITS)
            || fixed_width(minor, VERSION_TXT_MINOR_DIGITS_SHOWN))
    {
        return None;
    }

    let major = major.trim_start_matches('0');
    Some(format!(
        "{}.{}",
        if major.is_empty() { "0" } else { major },
        &minor[..VERSION_TXT_MINOR_DIGITS_SHOWN]
    ))
}

/// Required module stems under `sys/internal/`.
///
/// The directory scan already admits every `.sprx`. This list makes
/// absence of a stem fatal and admits a matching pre-decrypted `.prx`
/// that the scan did not find.
pub const FIRMWARE_INTERNAL_PRX_STEMS: &[&str] = &["libfs_utility2"];

/// Module stems shipped in retail firmware's `sys/external/`.
///
/// The set is the `.sprx` stems of that directory, so any retail
/// `dev_flash` install checks it. The list is a union across firmware
/// revisions: some revisions ship a module others do not, `libfs_155`
/// for one. Sorted for binary search.
pub const FIRMWARE_MODULE_STEMS: &[&str] = &[
    "libaacenc",
    "libaacenc_spurs",
    "libac3dec",
    "libac3dec2",
    "libad_async",
    "libad_billboard_util",
    "libad_core",
    "libadec",
    "libadec2",
    "libadec_internal",
    "libapostsrc_mini",
    "libasfparser2_astd",
    "libat3dec",
    "libat3multidec",
    "libatrac3multi",
    "libatrac3plus",
    "libatxdec",
    "libatxdec2",
    "libaudio",
    "libavcdec",
    "libavcenc",
    "libavcenc_small",
    "libavchatjpgdec",
    "libbeisobmf",
    "libbemp2sys",
    "libcamera",
    "libcelp8dec",
    "libcelp8enc",
    "libcelpdec",
    "libcelpenc",
    "libddpdec",
    "libdivxdec",
    "libdmux",
    "libdmuxpamf",
    "libdtslbrdec",
    "libfiber",
    "libfont",
    "libfontFT",
    "libfreetype",
    "libfreetypeTT",
    "libfs",
    "libfs_155",
    "libgcm_sys",
    "libgem",
    "libgifdec",
    "libhttp",
    "libio",
    "libjpgdec",
    "libjpgenc",
    "libkey2char",
    "libl10n",
    "liblv2",
    "liblv2coredump",
    "liblv2dbg_for_cex",
    "libm2bcdec",
    "libm4aacdec",
    "libm4aacdec2ch",
    "libm4hdenc",
    "libm4venc",
    "libmedi",
    "libmic",
    "libmp3dec",
    "libmp4",
    "libmpl1dec",
    "libmvcdec",
    "libnet",
    "libnetctl",
    "libpamf",
    "libpngdec",
    "libpngenc",
    "libresc",
    "librtc",
    "librudp",
    "libsail",
    "libsail_avi",
    "libsail_rec",
    "libsjvtd",
    "libsmvd2",
    "libsmvd4",
    "libspurs_jq",
    "libsre",
    "libssl",
    "libsvc1d",
    "libsync2",
    "libsysmodule",
    "libsysutil",
    "libsysutil_ap",
    "libsysutil_authdialog",
    "libsysutil_avc2",
    "libsysutil_avc_ext",
    "libsysutil_avconf_ext",
    "libsysutil_bgdl",
    "libsysutil_cross_controller",
    "libsysutil_dec_psnvideo",
    "libsysutil_dtcp_ip",
    "libsysutil_game",
    "libsysutil_game_exec",
    "libsysutil_imejp",
    "libsysutil_misc",
    "libsysutil_music",
    "libsysutil_music_decode",
    "libsysutil_music_export",
    "libsysutil_np",
    "libsysutil_np2",
    "libsysutil_np_clans",
    "libsysutil_np_commerce2",
    "libsysutil_np_eula",
    "libsysutil_np_installer",
    "libsysutil_np_sns",
    "libsysutil_np_trophy",
    "libsysutil_np_tus",
    "libsysutil_np_util",
    "libsysutil_oskdialog_ext",
    "libsysutil_pesm",
    "libsysutil_photo_decode",
    "libsysutil_photo_export",
    "libsysutil_photo_export2",
    "libsysutil_photo_import",
    "libsysutil_photo_network_sharing",
    "libsysutil_print",
    "libsysutil_rec",
    "libsysutil_remoteplay",
    "libsysutil_rtcalarm",
    "libsysutil_savedata",
    "libsysutil_savedata_psp",
    "libsysutil_screenshot",
    "libsysutil_search",
    "libsysutil_storagedata",
    "libsysutil_subdisplay",
    "libsysutil_syschat",
    "libsysutil_sysconf_ext",
    "libsysutil_userinfo",
    "libsysutil_video_export",
    "libsysutil_video_player",
    "libsysutil_video_upload",
    "libusbd",
    "libusbpspcm",
    "libvdec",
    "libvoice",
    "libvpost",
    "libvpost2",
    "libwmadec",
];

#[cfg(test)]
#[path = "tests/dev_flash_tests.rs"]
mod tests;
