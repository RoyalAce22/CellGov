//! Every store-mutating entry point refuses a pre-store root.
//!
//! [`super::preflight`] has its own cases beside the detector; these
//! prove each entry point calls it.
//!
//! Each case reaches the refusal before the installer parses its
//! container, so every case passes empty container bytes.

use crate::game_uninstall::{uninstall, UninstallOptions};
use crate::scratch_dir::{scratch, ScratchDir};

/// Placeholder identity: no case reaches a container, so no case names
/// an installed title.
const SYNTHETIC_TITLE_ID: &str = "TEST00000";

/// No case reaches the teardown, so no option here has an effect.
const PLAIN_UNINSTALL: UninstallOptions = UninstallOptions {
    verify: false,
    keep_rap: false,
    force: false,
};

/// A scratch root that holds the pre-store firmware mount.
fn pre_store_root() -> ScratchDir {
    let dir = scratch();
    std::fs::create_dir_all(dir.join("dev_flash")).expect("create the pre-store mount");
    dir
}

/// A clean scratch root. The paired case for each entry point uses it
/// to show that the layout gate produced the refusal.
fn store_root() -> ScratchDir {
    let dir = scratch();
    std::fs::create_dir_all(crate::store::StoreLayout::new(&*dir).installs_dir())
        .expect("create the records directory");
    dir
}

#[test]
fn uninstall_refuses_a_pre_store_root_before_it_reads_a_record() {
    let dir = pre_store_root();
    let err = uninstall(SYNTHETIC_TITLE_ID, &dir, PLAIN_UNINSTALL)
        .expect_err("a pre-store root is refused");
    assert!(
        matches!(err, crate::game_uninstall::GameUninstallError::PreStore(_)),
        "got {err:?}"
    );
}

#[test]
fn uninstall_on_a_store_root_reaches_the_record_read() {
    let dir = store_root();
    let err =
        uninstall(SYNTHETIC_TITLE_ID, &dir, PLAIN_UNINSTALL).expect_err("nothing is installed");
    assert!(
        matches!(
            err,
            crate::game_uninstall::GameUninstallError::NoRecord { .. }
        ),
        "got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
mod installers {
    use super::{pre_store_root, store_root};
    use crate::firmware_install::{install_pup, FirmwareInstallError};
    use crate::game_install::{
        install_iso, install_pkg, install_update_pkg, GameInstallError, InstallOptions,
    };
    use crate::test_support::{synthetic_vault, RecordingReporter};

    #[test]
    fn firmware_install_refuses_a_pre_store_root_before_it_parses_the_pup() {
        let dir = pre_store_root();
        let reporter = RecordingReporter::default();
        let err = install_pup(&[], &synthetic_vault(), &dir, false, &reporter)
            .expect_err("a pre-store root is refused");
        assert!(
            matches!(err, FirmwareInstallError::PreStore(_)),
            "got {err:?}"
        );
        assert!(
            reporter.phases().is_empty(),
            "the refusal precedes the first phase, so no progress is reported"
        );
    }

    #[test]
    fn firmware_install_on_a_store_root_reaches_the_pup_parse() {
        let dir = store_root();
        let reporter = RecordingReporter::default();
        let err = install_pup(&[], &synthetic_vault(), &dir, false, &reporter)
            .expect_err("an empty PUP is refused");
        assert!(matches!(err, FirmwareInstallError::Pup(_)), "got {err:?}");
    }

    #[test]
    fn title_install_refuses_a_pre_store_root_before_it_parses_the_pkg() {
        let dir = pre_store_root();
        let err = install_pkg(
            &[],
            None,
            &synthetic_vault(),
            &dir,
            InstallOptions::default(),
        )
        .expect_err("a pre-store root is refused");
        assert!(matches!(err, GameInstallError::PreStore(_)), "got {err:?}");
    }

    #[test]
    fn disc_install_refuses_a_pre_store_root_before_it_reads_the_image() {
        let dir = pre_store_root();
        let err = install_iso(&[], &synthetic_vault(), &dir, InstallOptions::default())
            .expect_err("a pre-store root is refused");
        assert!(matches!(err, GameInstallError::PreStore(_)), "got {err:?}");
    }

    #[test]
    fn update_install_refuses_a_pre_store_root_before_it_parses_the_pkg() {
        let dir = pre_store_root();
        let err = install_update_pkg(&[], &synthetic_vault(), &dir, InstallOptions::default())
            .expect_err("a pre-store root is refused");
        assert!(matches!(err, GameInstallError::PreStore(_)), "got {err:?}");
    }

    #[test]
    fn title_install_on_a_store_root_reaches_the_pkg_parse() {
        let dir = store_root();
        let err = install_pkg(
            &[],
            None,
            &synthetic_vault(),
            &dir,
            InstallOptions::default(),
        )
        .expect_err("an empty PKG is refused");
        assert!(matches!(err, GameInstallError::Pkg(_)), "got {err:?}");
    }
}
