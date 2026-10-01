//! The console before a run and after it: the preflight that refuses a
//! stale result or an occupied game directory, and the cleanup that
//! unmounts the test and removes everything the runner put on the
//! console.
//!
//! Both steps run against [`ConsoleOps`], which [`WebmanConsole`]
//! implements over the network and a test implements in memory.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::error::RunnerPs3Error;
use crate::transcript::Transcript;
use crate::transport::ftp::FtpSession;
use crate::transport::{http, Endpoint, TransportError};

/// Where installed games live on the console.
pub const GAME_ROOT: &str = "/dev_hdd0/game";
/// Where a microtest writes its result file.
pub const RESULT_ROOT: &str = "/dev_hdd0/tmp";
/// The webMAN request that unmounts the running game.
pub const UNMOUNT_PATH: &str = "/mount.ps3/unmount";

/// The console-side paths of one microtest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// `/dev_hdd0/game/<APPID>`.
    pub game_dir: String,
    /// `/dev_hdd0/game/<APPID>/USRDIR`.
    pub usrdir: String,
    /// `/dev_hdd0/tmp/<result file>`.
    pub result_path: String,
}

impl Target {
    /// The paths for a package installed under `appid` that writes
    /// `result_file`. Both come from the manifest, which refuses any
    /// value that is not a bare name.
    pub fn new(appid: &str, result_file: &str) -> Self {
        let game_dir = format!("{GAME_ROOT}/{appid}");
        Self {
            usrdir: format!("{game_dir}/USRDIR"),
            game_dir,
            result_path: format!("{RESULT_ROOT}/{result_file}"),
        }
    }

    /// The game directory's name under [`GAME_ROOT`].
    pub fn appid(&self) -> &str {
        base_name(&self.game_dir)
    }
}

/// The console operations preflight and cleanup need.
pub trait ConsoleOps {
    /// The HTTP status of `GET path`.
    ///
    /// # Errors
    ///
    /// Any transport failure.
    fn http_status(
        &mut self,
        path: &str,
        transcript: &mut Transcript,
    ) -> Result<u16, TransportError>;

    /// The entries the server lists for directory `dir`, as it lists
    /// them. webMAN answers a directory that does not exist with an empty
    /// listing, so an empty answer says nothing about existence; the
    /// parent's listing does.
    ///
    /// # Errors
    ///
    /// Any transport failure.
    fn list(
        &mut self,
        dir: &str,
        transcript: &mut Transcript,
    ) -> Result<Vec<String>, TransportError>;

    /// Delete the file at `path`.
    ///
    /// # Errors
    ///
    /// Any transport failure, the file's absence included.
    fn delete(&mut self, path: &str, transcript: &mut Transcript) -> Result<(), TransportError>;

    /// Remove the empty directory at `path`.
    ///
    /// # Errors
    ///
    /// Any transport failure.
    fn remove_dir(&mut self, path: &str, transcript: &mut Transcript)
        -> Result<(), TransportError>;
}

/// Refuse to run over a stale result or an occupied game directory.
///
/// The runner deletes a result file that answers `200` once and checks
/// it again; a second `200` is a refusal. An existing game directory is a refusal
/// unless `reclaim` is set, in which case [`cleanup`] empties it first.
/// Each refusal names `clear_with`, the command that clears it.
///
/// # Errors
///
/// [`RunnerPs3Error::Refused`], [`RunnerPs3Error::Transport`], or a
/// [`cleanup`] error from a reclaim.
pub fn preflight<C: ConsoleOps>(
    console: &mut C,
    target: &Target,
    reclaim: bool,
    clear_with: &str,
    transcript: &mut Transcript,
) -> Result<(), RunnerPs3Error> {
    if result_present(console, &target.result_path, transcript)? {
        transcript.decision(format!(
            "stale result at {}; deleting it once",
            target.result_path
        ));
        console.delete(&target.result_path, transcript)?;
        if result_present(console, &target.result_path, transcript)? {
            return Err(RunnerPs3Error::Refused {
                reason: format!(
                    "a result file remains at {} after one delete",
                    target.result_path
                ),
                clear_with: clear_with.to_string(),
            });
        }
    }
    if entries(console, GAME_ROOT, transcript)?
        .iter()
        .any(|name| name == target.appid())
    {
        if !reclaim {
            return Err(RunnerPs3Error::Refused {
                reason: format!("{} already exists on the console", target.game_dir),
                clear_with: format!("{clear_with}, or rerun with --reclaim"),
            });
        }
        transcript.decision(format!("reclaiming {}", target.game_dir));
        cleanup(console, target, transcript)?;
    }
    Ok(())
}

/// Unmount the test and remove the game directory and the result file.
/// Every step runs even after one fails, so the error names everything
/// that remains.
///
/// # Errors
///
/// [`RunnerPs3Error::Cleanup`] naming each path the runner could not
/// remove, or the unmount.
pub fn cleanup<C: ConsoleOps>(
    console: &mut C,
    target: &Target,
    transcript: &mut Transcript,
) -> Result<(), RunnerPs3Error> {
    let mut remaining = Vec::new();
    match console.http_status(UNMOUNT_PATH, transcript) {
        Ok(200) => {}
        Ok(_) | Err(_) => remaining.push("the mounted game (unmount failed)".to_string()),
    }
    match entries(console, GAME_ROOT, transcript) {
        Ok(games) if games.iter().any(|name| name == target.appid()) => {
            remove_game_dir(console, target, &mut remaining, transcript);
        }
        Ok(_) => {}
        Err(_) => remaining.push(target.game_dir.clone()),
    }
    match result_present(console, &target.result_path, transcript) {
        Ok(false) => {}
        Ok(true) => {
            if console.delete(&target.result_path, transcript).is_err() {
                remaining.push(target.result_path.clone());
            }
        }
        Err(_) => remaining.push(target.result_path.clone()),
    }
    if remaining.is_empty() {
        transcript.decision("console restored");
        Ok(())
    } else {
        transcript.decision(format!("cleanup left: {}", remaining.join(", ")));
        Err(RunnerPs3Error::Cleanup {
            remaining: remaining.join(", "),
        })
    }
}

/// Empty and remove the game directory, which its parent lists: its
/// `USRDIR` first when the directory lists one, then every other entry,
/// then the directory.
fn remove_game_dir<C: ConsoleOps>(
    console: &mut C,
    target: &Target,
    remaining: &mut Vec<String>,
    transcript: &mut Transcript,
) {
    let names = match entries(console, &target.game_dir, transcript) {
        Ok(names) => names,
        Err(_) => {
            remaining.push(target.game_dir.clone());
            return;
        }
    };
    for name in names {
        if name == "USRDIR" {
            empty_and_remove(console, &target.usrdir, remaining, transcript);
        } else {
            delete_or_record(
                console,
                &format!("{}/{name}", target.game_dir),
                remaining,
                transcript,
            );
        }
    }
    if console.remove_dir(&target.game_dir, transcript).is_err() {
        remaining.push(target.game_dir.clone());
    }
}

/// Delete every entry directly in `dir`, which its parent lists, then
/// `dir`. A subdirectory fails its delete and so stays in `remaining`.
fn empty_and_remove<C: ConsoleOps>(
    console: &mut C,
    dir: &str,
    remaining: &mut Vec<String>,
    transcript: &mut Transcript,
) {
    match entries(console, dir, transcript) {
        Ok(names) => {
            for name in names {
                delete_or_record(console, &format!("{dir}/{name}"), remaining, transcript);
            }
        }
        Err(_) => remaining.push(format!("the contents of {dir}")),
    }
    if console.remove_dir(dir, transcript).is_err() {
        remaining.push(dir.to_string());
    }
}

fn delete_or_record<C: ConsoleOps>(
    console: &mut C,
    path: &str,
    remaining: &mut Vec<String>,
    transcript: &mut Transcript,
) {
    if console.delete(path, transcript).is_err() {
        remaining.push(path.to_string());
    }
}

/// The names in `dir`, by last path component, without the `.` and `..`
/// entries webMAN lists.
fn entries<C: ConsoleOps>(
    console: &mut C,
    dir: &str,
    transcript: &mut Transcript,
) -> Result<Vec<String>, TransportError> {
    Ok(console
        .list(dir, transcript)?
        .iter()
        .map(|entry| base_name(entry))
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .map(str::to_string)
        .collect())
}

fn result_present<C: ConsoleOps>(
    console: &mut C,
    path: &str,
    transcript: &mut Transcript,
) -> Result<bool, TransportError> {
    match console.http_status(path, transcript)? {
        200 => Ok(true),
        404 => Ok(false),
        status => Err(TransportError::UnexpectedStatus {
            path: path.to_string(),
            status,
        }),
    }
}

/// The last path component of a listing entry; a server may list full
/// paths or bare names.
fn base_name(entry: &str) -> &str {
    entry
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(entry)
}

/// The console over webMAN's HTTP and FTP servers, one connection per
/// operation.
#[derive(Debug, Clone)]
pub struct WebmanConsole {
    endpoint: Endpoint,
}

impl WebmanConsole {
    /// The console at `endpoint`.
    pub fn new(endpoint: Endpoint) -> Self {
        Self { endpoint }
    }

    fn connect(&self, address: SocketAddr) -> Result<TcpStream, TransportError> {
        let timeout = Duration::from_millis(self.endpoint.io_timeout_ms);
        let io = |source| TransportError::Io {
            operation: "connect",
            source,
        };
        let stream = TcpStream::connect_timeout(&address, timeout).map_err(io)?;
        stream.set_read_timeout(Some(timeout)).map_err(io)?;
        stream.set_write_timeout(Some(timeout)).map_err(io)?;
        Ok(stream)
    }

    fn connect_port(&self, port: u16) -> Result<TcpStream, TransportError> {
        let address = (self.endpoint.host.as_str(), port)
            .to_socket_addrs()
            .map_err(|source| TransportError::Io {
                operation: "resolve",
                source,
            })?
            .next()
            .ok_or_else(|| TransportError::Io {
                operation: "resolve",
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("{} has no address", self.endpoint.host),
                ),
            })?;
        self.connect(address)
    }

    fn ftp(&self, transcript: &mut Transcript) -> Result<FtpSession<TcpStream>, TransportError> {
        let control = self.connect_port(self.endpoint.ftp_port)?;
        let mut session = FtpSession::open(control, transcript)?;
        session.login_anonymous(transcript)?;
        session.type_binary(transcript)?;
        Ok(session)
    }
}

impl ConsoleOps for WebmanConsole {
    fn http_status(
        &mut self,
        path: &str,
        transcript: &mut Transcript,
    ) -> Result<u16, TransportError> {
        let mut stream = self.connect_port(self.endpoint.http_port)?;
        http::get(&mut stream, &self.endpoint.host, path, transcript).map(|r| r.status)
    }

    fn list(
        &mut self,
        dir: &str,
        transcript: &mut Transcript,
    ) -> Result<Vec<String>, TransportError> {
        let mut session = self.ftp(transcript)?;
        let data = self.connect(SocketAddr::V4(session.pasv(transcript)?))?;
        let names = session.nlst(dir, data, transcript)?;
        session.quit(transcript)?;
        Ok(names)
    }

    fn delete(&mut self, path: &str, transcript: &mut Transcript) -> Result<(), TransportError> {
        let mut session = self.ftp(transcript)?;
        session.dele(path, transcript)?;
        session.quit(transcript).map(drop)
    }

    fn remove_dir(
        &mut self,
        path: &str,
        transcript: &mut Transcript,
    ) -> Result<(), TransportError> {
        let mut session = self.ftp(transcript)?;
        session.rmd(path, transcript)?;
        session.quit(transcript).map(drop)
    }
}

#[cfg(test)]
#[path = "tests/run_tests.rs"]
mod tests;
