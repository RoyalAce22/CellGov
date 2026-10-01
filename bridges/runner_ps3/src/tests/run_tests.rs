//! Preflight and cleanup against an in-memory console: the stale-result
//! rule, the occupied game directory, reclaim, and a cleanup that names
//! everything it could not remove.

use std::collections::BTreeSet;

use super::*;

/// A console holding `files` and `dirs` by full path. `stubborn` paths
/// refuse deletion; `respawn` puts the result file back after a delete.
#[derive(Default)]
struct MemoryConsole {
    files: BTreeSet<String>,
    dirs: BTreeSet<String>,
    stubborn: BTreeSet<String>,
    respawn: bool,
    unmounts: usize,
    unmount_status: Option<u16>,
    calls: Vec<String>,
}

fn refused(path: &str) -> TransportError {
    TransportError::UnexpectedReply {
        command: format!("DELE {path}"),
        code: 550,
        text: "Permission denied".to_string(),
    }
}

impl MemoryConsole {
    fn with_package(target: &Target) -> Self {
        let mut console = Self::default();
        console.dirs.insert(GAME_ROOT.to_string());
        console.dirs.insert(target.game_dir.clone());
        console.dirs.insert(target.usrdir.clone());
        console
            .files
            .insert(format!("{}/PARAM.SFO", target.game_dir));
        console.files.insert(format!("{}/EBOOT.BIN", target.usrdir));
        console
            .files
            .insert(format!("{}/spu_main.elf", target.usrdir));
        console
    }

    fn empty() -> Self {
        let mut console = Self::default();
        console.dirs.insert(GAME_ROOT.to_string());
        console
    }
}

impl ConsoleOps for MemoryConsole {
    fn http_status(&mut self, path: &str, _: &mut Transcript) -> Result<u16, TransportError> {
        self.calls.push(format!("GET {path}"));
        if path == UNMOUNT_PATH {
            self.unmounts += 1;
            return Ok(self.unmount_status.unwrap_or(200));
        }
        Ok(if self.files.contains(path) { 200 } else { 404 })
    }

    fn list(&mut self, dir: &str, _: &mut Transcript) -> Result<Vec<String>, TransportError> {
        // As webMAN answers, measured on the console: a directory that
        // does not exist lists empty, and one that does lists `.` and
        // `..` before its bare names.
        self.calls.push(format!("NLST {dir}"));
        if !self.dirs.contains(dir) {
            return Ok(Vec::new());
        }
        let prefix = format!("{dir}/");
        let children = self
            .files
            .iter()
            .chain(&self.dirs)
            .filter_map(|p| p.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('/'))
            .map(str::to_string);
        Ok([".".to_string(), "..".to_string()]
            .into_iter()
            .chain(children)
            .collect())
    }

    fn delete(&mut self, path: &str, _: &mut Transcript) -> Result<(), TransportError> {
        self.calls.push(format!("DELE {path}"));
        if self.stubborn.contains(path) || !self.files.remove(path) {
            return Err(refused(path));
        }
        if self.respawn {
            self.files.insert(path.to_string());
        }
        Ok(())
    }

    fn remove_dir(&mut self, path: &str, _: &mut Transcript) -> Result<(), TransportError> {
        self.calls.push(format!("RMD {path}"));
        let prefix = format!("{path}/");
        let occupied = self
            .files
            .iter()
            .chain(&self.dirs)
            .any(|p| p.starts_with(&prefix));
        if occupied || !self.dirs.remove(path) {
            return Err(refused(path));
        }
        Ok(())
    }
}

fn target() -> Target {
    Target::new("CGOV00001", "cgov_spu_fixed_value.bin")
}

const CLEAR: &str = "runner_ps3 cleanup --host 10.77.0.2";

#[test]
fn the_target_paths_follow_the_appid_and_the_result_file() {
    assert_eq!(
        target(),
        Target {
            game_dir: "/dev_hdd0/game/CGOV00001".to_string(),
            usrdir: "/dev_hdd0/game/CGOV00001/USRDIR".to_string(),
            result_path: "/dev_hdd0/tmp/cgov_spu_fixed_value.bin".to_string(),
        }
    );
}

#[test]
fn a_clean_console_passes_preflight_without_a_change() {
    let mut console = MemoryConsole::empty();
    let mut transcript = Transcript::new();
    preflight(&mut console, &target(), false, CLEAR, &mut transcript).expect("clean");
    assert!(!console
        .calls
        .iter()
        .any(|c| c.starts_with("DELE") || c.starts_with("RMD")));
}

#[test]
fn a_stale_result_is_deleted_once_and_the_run_proceeds() {
    let mut console = MemoryConsole::empty();
    console.files.insert(target().result_path);
    let mut transcript = Transcript::new();
    preflight(&mut console, &target(), false, CLEAR, &mut transcript).expect("cleared");
    assert!(console.files.is_empty());
    assert_eq!(
        console
            .calls
            .iter()
            .filter(|c| c.starts_with("DELE"))
            .count(),
        1
    );
}

#[test]
fn a_result_that_survives_its_delete_is_refused_with_the_clearing_command() {
    let mut console = MemoryConsole::empty();
    console.files.insert(target().result_path);
    console.respawn = true;
    let mut transcript = Transcript::new();
    let err =
        preflight(&mut console, &target(), false, CLEAR, &mut transcript).expect_err("refused");
    match err {
        RunnerPs3Error::Refused { reason, clear_with } => {
            assert!(
                reason.contains("/dev_hdd0/tmp/cgov_spu_fixed_value.bin"),
                "{reason}"
            );
            assert_eq!(clear_with, CLEAR);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        console
            .calls
            .iter()
            .filter(|c| c.starts_with("DELE"))
            .count(),
        1,
        "the rule deletes once"
    );
}

#[test]
fn an_occupied_game_directory_is_refused_unless_reclaimed() {
    let mut console = MemoryConsole::with_package(&target());
    let mut transcript = Transcript::new();
    let err =
        preflight(&mut console, &target(), false, CLEAR, &mut transcript).expect_err("occupied");
    match &err {
        RunnerPs3Error::Refused { reason, clear_with } => {
            assert_eq!(
                reason,
                "/dev_hdd0/game/CGOV00001 already exists on the console"
            );
            assert!(clear_with.ends_with("--reclaim"), "{clear_with}");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        console.dirs.contains(&target().game_dir),
        "a refusal changes nothing"
    );

    preflight(&mut console, &target(), true, CLEAR, &mut transcript).expect("reclaimed");
    assert!(!console.dirs.contains(&target().game_dir));
    assert!(console.files.is_empty());
}

#[test]
fn cleanup_unmounts_and_removes_the_package_and_the_result() {
    let mut console = MemoryConsole::with_package(&target());
    console.files.insert(target().result_path);
    let mut transcript = Transcript::new();
    cleanup(&mut console, &target(), &mut transcript).expect("restored");
    assert_eq!(console.unmounts, 1);
    assert!(console.files.is_empty(), "{:?}", console.files);
    assert_eq!(console.dirs.iter().collect::<Vec<_>>(), [GAME_ROOT]);
    assert_eq!(
        transcript.lines().last().map(String::as_str),
        Some("#0001 = console restored")
    );
}

#[test]
fn cleanup_of_an_empty_console_is_a_no_op_that_still_unmounts() {
    let mut console = MemoryConsole::empty();
    let mut transcript = Transcript::new();
    cleanup(&mut console, &target(), &mut transcript).expect("nothing to remove");
    assert_eq!(console.unmounts, 1);
    assert!(
        !console
            .calls
            .iter()
            .any(|c| c.starts_with("DELE") || c.starts_with("RMD")),
        "a game directory its parent does not list is never touched: {:?}",
        console.calls
    );
}

#[test]
fn the_dot_entries_a_listing_carries_are_never_deleted() {
    let mut console = MemoryConsole::with_package(&target());
    let mut transcript = Transcript::new();
    cleanup(&mut console, &target(), &mut transcript).expect("restored");
    let deletes: Vec<&String> = console
        .calls
        .iter()
        .filter(|c| c.starts_with("DELE"))
        .collect();
    assert_eq!(
        deletes,
        [
            "DELE /dev_hdd0/game/CGOV00001/PARAM.SFO",
            "DELE /dev_hdd0/game/CGOV00001/USRDIR/EBOOT.BIN",
            "DELE /dev_hdd0/game/CGOV00001/USRDIR/spu_main.elf",
        ]
    );
}

#[test]
fn a_cleanup_that_cannot_finish_names_everything_that_remains_and_keeps_going() {
    let mut console = MemoryConsole::with_package(&target());
    console.files.insert(target().result_path);
    console
        .stubborn
        .insert(format!("{}/EBOOT.BIN", target().usrdir));
    console.unmount_status = Some(500);
    let mut transcript = Transcript::new();
    let err = cleanup(&mut console, &target(), &mut transcript).expect_err("incomplete");
    assert_eq!(err.exit_code(), crate::error::ExitCode::Cleanup);
    match err {
        RunnerPs3Error::Cleanup { remaining } => assert_eq!(
            remaining,
            "the mounted game (unmount failed), \
             /dev_hdd0/game/CGOV00001/USRDIR/EBOOT.BIN, \
             /dev_hdd0/game/CGOV00001/USRDIR, \
             /dev_hdd0/game/CGOV00001"
        ),
        other => panic!("{other:?}"),
    }
    assert!(
        !console.files.contains(&target().result_path),
        "later steps still ran"
    );
    assert!(!console
        .files
        .contains(&format!("{}/spu_main.elf", target().usrdir)));
}

#[test]
fn a_result_path_that_answers_neither_200_nor_404_is_a_transport_error() {
    struct Teapot;
    impl ConsoleOps for Teapot {
        fn http_status(&mut self, _: &str, _: &mut Transcript) -> Result<u16, TransportError> {
            Ok(418)
        }
        fn list(&mut self, _: &str, _: &mut Transcript) -> Result<Vec<String>, TransportError> {
            Ok(Vec::new())
        }
        fn delete(&mut self, path: &str, _: &mut Transcript) -> Result<(), TransportError> {
            Err(refused(path))
        }
        fn remove_dir(&mut self, path: &str, _: &mut Transcript) -> Result<(), TransportError> {
            Err(refused(path))
        }
    }
    let err =
        preflight(&mut Teapot, &target(), false, CLEAR, &mut Transcript::new()).expect_err("418");
    assert!(
        matches!(
            err,
            RunnerPs3Error::Transport(TransportError::UnexpectedStatus { status: 418, .. })
        ),
        "{err:?}"
    );
}
