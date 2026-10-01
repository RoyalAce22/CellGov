//! Deployment of a packaged microtest to the console: the EBOOT, its
//! PARAM.SFO and the siblings the manifest declares, under the game
//! directory of the manifest's appid.
//!
//! The package is what `tests/micro/common/package_ps3.sh` leaves in
//! `build/ps3/`. The siblings go into `USRDIR` beside the EBOOT, the
//! directory webMAN mounts as `/app_home`.

use std::path::{Path, PathBuf};

use cellgov_observation::manifest::ConsoleManifest;

use crate::error::RunnerPs3Error;
use crate::run::{ConsoleOps, Target};
use crate::transcript::Transcript;

/// The packaged EBOOT's file name.
pub const EBOOT: &str = "EBOOT.BIN";
/// The package's parameter file name.
pub const PARAM_SFO: &str = "PARAM.SFO";

/// The files of one packaged microtest, on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// The microtest directory, the manifest's.
    pub test_dir: PathBuf,
    /// `build/ps3/EBOOT.BIN`.
    pub eboot: PathBuf,
    /// `build/ps3/PARAM.SFO`.
    pub param_sfo: PathBuf,
    /// `build/ps3/<name>.elf`, the relink the EBOOT wraps.
    pub ps3_elf: PathBuf,
    /// `build/<name>.elf`, the reference the emulators run.
    pub reference_elf: PathBuf,
    /// Each `[ps3] files` entry, by name, under `build/ps3/`.
    pub siblings: Vec<(String, PathBuf)>,
}

impl Package {
    /// The package of the microtest whose manifest sits at
    /// `manifest_path`.
    pub fn of(manifest_path: &Path, manifest: &ConsoleManifest) -> Self {
        let test_dir = manifest_path
            .parent()
            .map_or_else(PathBuf::new, Path::to_path_buf);
        let build = test_dir.join("build");
        let ps3 = build.join("ps3");
        let name = &manifest.test.name;
        Self {
            eboot: ps3.join(EBOOT),
            param_sfo: ps3.join(PARAM_SFO),
            ps3_elf: ps3.join(format!("{name}.elf")),
            reference_elf: build.join(format!("{name}.elf")),
            siblings: manifest
                .ps3
                .files
                .iter()
                .map(|file| (file.clone(), ps3.join(file)))
                .collect(),
            test_dir,
        }
    }
}

fn read(path: &Path) -> Result<Vec<u8>, RunnerPs3Error> {
    std::fs::read(path).map_err(|source| RunnerPs3Error::LocalRead {
        path: path.to_path_buf(),
        source,
    })
}

/// Create the game directory and `USRDIR`, then store the PARAM.SFO in
/// the first and the EBOOT and every sibling in the second. Every local
/// file is read before anything is sent, so a missing build output
/// changes nothing on the console.
///
/// # Errors
///
/// [`RunnerPs3Error::LocalRead`] naming the first package file the
/// runner cannot read, and [`RunnerPs3Error::Transport`] for a failed
/// exchange.
pub fn deploy<C: ConsoleOps>(
    console: &mut C,
    target: &Target,
    package: &Package,
    transcript: &mut Transcript,
) -> Result<(), RunnerPs3Error> {
    let mut usrdir_files = vec![(EBOOT.to_string(), read(&package.eboot)?)];
    for (name, path) in &package.siblings {
        usrdir_files.push((name.clone(), read(path)?));
    }
    let param_sfo = read(&package.param_sfo)?;
    console.make_dir(&target.game_dir, transcript)?;
    console.make_dir(&target.usrdir, transcript)?;
    console.store(
        &format!("{}/{PARAM_SFO}", target.game_dir),
        &param_sfo,
        transcript,
    )?;
    for (name, bytes) in &usrdir_files {
        console.store(&format!("{}/{name}", target.usrdir), bytes, transcript)?;
    }
    transcript.decision(format!(
        "deployed {} file(s) under {}",
        usrdir_files.len() + 1,
        target.game_dir
    ));
    Ok(())
}

#[cfg(test)]
#[path = "tests/deploy_tests.rs"]
mod tests;
