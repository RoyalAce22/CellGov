//! An in-memory console for the runner's tests, answering as webMAN
//! does where the console's answers were measured: a directory that
//! does not exist lists empty, one that does lists `.` and `..` before
//! its bare names, and it answers `MKD` on a directory that exists with
//! `550`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::console::STATUS_PATH;
use crate::run::{ConsoleOps, Target, GAME_ROOT, PLAY_PATH, RESULT_ROOT, UNMOUNT_PATH};
use crate::transcript::Transcript;
use crate::transport::TransportError;

/// A console holding `files` and `dirs` by full path.
#[derive(Default)]
pub(crate) struct MemoryConsole {
    /// Each file's bytes, by full path.
    pub(crate) files: BTreeMap<String, Vec<u8>>,
    /// Each directory, by full path.
    pub(crate) dirs: BTreeSet<String>,
    /// Paths the console refuses to delete.
    pub(crate) stubborn: BTreeSet<String>,
    /// Puts a deleted file back.
    pub(crate) respawn: bool,
    /// How many unmount requests the console answered.
    pub(crate) unmounts: usize,
    /// The unmount request's status; `200` when unset.
    pub(crate) unmount_status: Option<u16>,
    /// What a started test writes: its path and bytes.
    pub(crate) on_start: Option<(String, Vec<u8>)>,
    /// How many result polls answer `404` after the start before the
    /// test's file appears.
    pub(crate) polls_before_result: usize,
    /// The status pages the console serves after the current one, one
    /// per read of the status page.
    pub(crate) later_status: VecDeque<Vec<u8>>,
    /// How many status-page reads the console refuses, as it does while
    /// the XMB reloads after a title exits.
    pub(crate) status_refusals: usize,
    /// How many start requests the console answers without starting the
    /// title, as it does soon after a previous title exits.
    pub(crate) ignored_starts: usize,
    /// How many result reads, once the file exists, find it still being
    /// written: the body outgrows webMAN's stated length.
    pub(crate) growing_reads: usize,
    /// The path `growing_reads` applies to.
    pub(crate) result_path: Option<String>,
    /// Every request, in order.
    pub(crate) calls: Vec<String>,
    started: bool,
}

/// The manifest of the packaged test [`package_on_disk`] writes.
pub(crate) const PACKAGED_MANIFEST: &str = r#"[test]
name = "spu_fixed_value"

[observe]
memory_regions = [
  { name = "result", addr = 0, size = 8 },
]

[expect]
outcome = "completed"

[ps3]
files = ["spu_main.elf"]
"#;

/// The frame the packaged test writes: status 0, value `0x1337BAAD`.
pub(crate) const PACKAGED_FRAME: &[u8] = b"CGOV\x00\x00\x00\x08\x00\x00\x00\x00\x13\x37\xBA\xAD";

/// A packaged `spu_fixed_value` under `root`, each build file holding
/// its own name; returns the manifest path.
pub(crate) fn package_on_disk(root: &std::path::Path) -> std::path::PathBuf {
    let dir = root.join("spu_fixed_value");
    let ps3 = dir.join("build").join("ps3");
    std::fs::create_dir_all(&ps3).expect("package dirs");
    std::fs::create_dir_all(dir.join("ppu")).expect("source dir");
    let manifest = dir.join("manifest.toml");
    std::fs::write(&manifest, PACKAGED_MANIFEST).expect("manifest");
    std::fs::write(dir.join("ppu").join("main.c"), "int main(void);").expect("source");
    for name in [
        "EBOOT.BIN",
        "PARAM.SFO",
        "spu_fixed_value.elf",
        "spu_main.elf",
    ] {
        std::fs::write(ps3.join(name), name).expect("package file");
    }
    std::fs::write(dir.join("build").join("spu_fixed_value.elf"), "reference").expect("reference");
    manifest
}

/// The refusal a stubborn path answers with.
pub(crate) fn refused(path: &str) -> TransportError {
    TransportError::UnexpectedReply {
        command: format!("DELE {path}"),
        code: 550,
        text: "Permission denied".to_string(),
    }
}

impl MemoryConsole {
    /// A console with nothing installed.
    pub(crate) fn empty() -> Self {
        let mut console = Self::default();
        console.dirs.insert(GAME_ROOT.to_string());
        console.dirs.insert(RESULT_ROOT.to_string());
        console
    }

    /// A console with `target`'s package installed.
    pub(crate) fn with_package(target: &Target) -> Self {
        let mut console = Self::empty();
        console.dirs.insert(target.game_dir.clone());
        console.dirs.insert(target.usrdir.clone());
        console.put(&format!("{}/PARAM.SFO", target.game_dir));
        console.put(&format!("{}/EBOOT.BIN", target.usrdir));
        console.put(&format!("{}/spu_main.elf", target.usrdir));
        console
    }

    /// An empty file at `path`.
    pub(crate) fn put(&mut self, path: &str) {
        self.files.insert(path.to_string(), Vec::new());
    }

    fn parent_exists(&self, path: &str) -> bool {
        path.rsplit_once('/')
            .is_some_and(|(parent, _)| self.dirs.contains(parent))
    }

    /// Whether `path` answers `200`; the started test's file appears
    /// once its polls have run out.
    fn poll(&mut self, path: &str) -> bool {
        let writes_here = self
            .on_start
            .as_ref()
            .is_some_and(|(result, _)| result == path);
        if self.started && writes_here {
            if self.polls_before_result > 0 {
                self.polls_before_result -= 1;
                return false;
            }
            if let Some((result, bytes)) = self.on_start.take() {
                self.files.insert(result, bytes);
            }
        }
        self.files.contains_key(path)
    }
}

impl ConsoleOps for MemoryConsole {
    fn http_status(&mut self, path: &str, _: &mut Transcript) -> Result<u16, TransportError> {
        self.calls.push(format!("GET {path}"));
        if path == UNMOUNT_PATH {
            self.unmounts += 1;
            self.started = false;
            return Ok(self.unmount_status.unwrap_or(200));
        }
        if let Some(appid) = path
            .strip_prefix(PLAY_PATH)
            .and_then(|rest| rest.strip_prefix('?'))
        {
            // webMAN answers 200 and starts the title when its EBOOT is
            // installed.
            if self.ignored_starts > 0 {
                self.ignored_starts -= 1;
                return Ok(200);
            }
            self.started = self
                .files
                .contains_key(&format!("{GAME_ROOT}/{appid}/USRDIR/EBOOT.BIN"));
            return Ok(200);
        }
        let present = self.poll(path);
        let result = self.result_path.as_deref() == Some(path);
        if present && result && self.growing_reads > 0 {
            self.growing_reads -= 1;
            return Err(TransportError::BodyLength {
                found: 16,
                expected: 4,
            });
        }
        Ok(if present { 200 } else { 404 })
    }

    fn fetch(&mut self, path: &str, _: &mut Transcript) -> Result<Option<Vec<u8>>, TransportError> {
        self.calls.push(format!("FETCH {path}"));
        if path == STATUS_PATH && self.status_refusals > 0 {
            self.status_refusals -= 1;
            return Err(TransportError::Io {
                operation: "connect",
                source: std::io::Error::from(std::io::ErrorKind::ConnectionAborted),
            });
        }
        let page = self.files.get(path).cloned();
        if path == STATUS_PATH {
            if let Some(next) = self.later_status.pop_front() {
                self.files.insert(path.to_string(), next);
            }
        }
        Ok(page)
    }

    fn list(&mut self, dir: &str, _: &mut Transcript) -> Result<Vec<String>, TransportError> {
        self.calls.push(format!("NLST {dir}"));
        if !self.dirs.contains(dir) {
            return Ok(Vec::new());
        }
        let prefix = format!("{dir}/");
        let children = self
            .files
            .keys()
            .chain(&self.dirs)
            .filter_map(|p| p.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('/'))
            .map(str::to_string);
        Ok([".".to_string(), "..".to_string()]
            .into_iter()
            .chain(children)
            .collect())
    }

    fn make_dir(&mut self, path: &str, _: &mut Transcript) -> Result<(), TransportError> {
        self.calls.push(format!("MKD {path}"));
        if !self.parent_exists(path) || !self.dirs.insert(path.to_string()) {
            // The reply the reference console gave a second MKD.
            return Err(TransportError::UnexpectedReply {
                command: format!("MKD {path}"),
                code: 550,
                text: format!("File Error \"{path}/\""),
            });
        }
        Ok(())
    }

    fn store(
        &mut self,
        path: &str,
        bytes: &[u8],
        _: &mut Transcript,
    ) -> Result<(), TransportError> {
        self.calls
            .push(format!("STOR {path} ({} bytes)", bytes.len()));
        if !self.parent_exists(path) {
            return Err(refused(path));
        }
        self.files.insert(path.to_string(), bytes.to_vec());
        Ok(())
    }

    fn delete(&mut self, path: &str, _: &mut Transcript) -> Result<(), TransportError> {
        self.calls.push(format!("DELE {path}"));
        if self.stubborn.contains(path) {
            return Err(refused(path));
        }
        let Some(bytes) = self.files.remove(path) else {
            return Err(refused(path));
        };
        if self.respawn {
            self.files.insert(path.to_string(), bytes);
        }
        Ok(())
    }

    fn remove_dir(&mut self, path: &str, _: &mut Transcript) -> Result<(), TransportError> {
        self.calls.push(format!("RMD {path}"));
        let prefix = format!("{path}/");
        let occupied = self
            .files
            .keys()
            .chain(&self.dirs)
            .any(|p| p.starts_with(&prefix));
        if occupied || !self.dirs.remove(path) {
            return Err(refused(path));
        }
        Ok(())
    }
}
