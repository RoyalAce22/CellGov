//! The console before, during and after a run: the preflight that
//! refuses a stale result or an occupied game directory, the start and
//! the polled wait for the result, the fetch, and the cleanup that
//! unmounts the test and removes everything the runner put on the
//! console.
//!
//! Every step runs against [`ConsoleOps`], which [`WebmanConsole`]
//! implements over the network and a test implements in memory.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::console::{at_xmb, STATUS_PATH};
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
/// The webMAN request that, followed by `?<APPID>`, mounts the game
/// installed under that directory of [`GAME_ROOT`] and starts it.
pub const PLAY_PATH: &str = "/play.ps3";
/// How long a start may go unanswered by a title before the runner sends
/// it once more.
pub const RESEND_START_AFTER_MS: u64 = 10_000;

/// The console-side paths of one microtest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// `/dev_hdd0/game/<APPID>`.
    pub game_dir: String,
    /// `/dev_hdd0/game/<APPID>/USRDIR`.
    pub usrdir: String,
    /// `/dev_hdd0/tmp/<result file>`, or `<usrdir>/<result file>` for a
    /// package that writes its result under its own `USRDIR`.
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

    /// The paths the manifest names: [`Self::new`], with the result
    /// under `USRDIR` when `[ps3] result_in_usrdir` asks for it. The
    /// runner fetches the result before cleanup removes the game
    /// directory.
    pub fn of(manifest: &cellgov_observation::manifest::ConsoleManifest) -> Self {
        let mut target = Self::new(&manifest.ps3.appid, &manifest.result_file_name());
        if manifest.ps3.result_in_usrdir {
            target.result_path = format!("{}/{}", target.usrdir, manifest.result_file_name());
        }
        target
    }

    /// The game directory's name under [`GAME_ROOT`].
    pub fn appid(&self) -> &str {
        base_name(&self.game_dir)
    }
}

/// The console operations the verbs need.
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

    /// The body of `GET path`, or `None` when it answers `404`.
    ///
    /// # Errors
    ///
    /// Any transport failure, and [`TransportError::UnexpectedStatus`]
    /// for a status other than `200` or `404`.
    fn fetch(
        &mut self,
        path: &str,
        transcript: &mut Transcript,
    ) -> Result<Option<Vec<u8>>, TransportError>;

    /// Create the directory `path`.
    ///
    /// # Errors
    ///
    /// Any transport failure.
    fn make_dir(&mut self, path: &str, transcript: &mut Transcript) -> Result<(), TransportError>;

    /// Store `bytes` as the file `path`.
    ///
    /// # Errors
    ///
    /// Any transport failure.
    fn store(
        &mut self,
        path: &str,
        bytes: &[u8],
        transcript: &mut Transcript,
    ) -> Result<(), TransportError>;

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

    /// Close the connection that the console keeps open between operations.
    ///
    /// The next operation opens a new connection. The runner ends the
    /// session at these points:
    ///
    /// - before [`start`];
    /// - before it waits on the operator;
    /// - when a verb ends.
    ///
    /// A console that keeps no connection open does nothing.
    fn end_session(&mut self, transcript: &mut Transcript) {
        let _ = transcript;
    }
}

/// A borrowed console is a console, so a caller that keeps its console
/// lends it to a verb.
impl<C: ConsoleOps + ?Sized> ConsoleOps for &mut C {
    fn http_status(
        &mut self,
        path: &str,
        transcript: &mut Transcript,
    ) -> Result<u16, TransportError> {
        (**self).http_status(path, transcript)
    }

    fn fetch(
        &mut self,
        path: &str,
        transcript: &mut Transcript,
    ) -> Result<Option<Vec<u8>>, TransportError> {
        (**self).fetch(path, transcript)
    }

    fn make_dir(&mut self, path: &str, transcript: &mut Transcript) -> Result<(), TransportError> {
        (**self).make_dir(path, transcript)
    }

    fn store(
        &mut self,
        path: &str,
        bytes: &[u8],
        transcript: &mut Transcript,
    ) -> Result<(), TransportError> {
        (**self).store(path, bytes, transcript)
    }

    fn list(
        &mut self,
        dir: &str,
        transcript: &mut Transcript,
    ) -> Result<Vec<String>, TransportError> {
        (**self).list(dir, transcript)
    }

    fn delete(&mut self, path: &str, transcript: &mut Transcript) -> Result<(), TransportError> {
        (**self).delete(path, transcript)
    }

    fn remove_dir(
        &mut self,
        path: &str,
        transcript: &mut Transcript,
    ) -> Result<(), TransportError> {
        (**self).remove_dir(path, transcript)
    }

    fn end_session(&mut self, transcript: &mut Transcript) {
        (**self).end_session(transcript);
    }
}

/// What `--reclaim` would remove: an occupied game directory, and every
/// path in it and in its `USRDIR`, as the console lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reclaim {
    /// The game directory.
    pub game_dir: String,
    /// Every path in it, `USRDIR`'s contents included.
    pub contents: Vec<String>,
}

/// Refuse to run over a stale result or an occupied game directory.
///
/// The runner deletes a result file that answers `200` once and checks
/// it again; a second `200` is a refusal. An existing game directory is
/// a refusal unless `reclaim` is set and `may_reclaim`, shown the
/// directory and what it holds, answers yes; then [`cleanup`] empties
/// it. A declined reclaim is a refusal that leaves the console as it
/// was. Each refusal names `clear_with`, the command that clears it.
///
/// # Errors
///
/// [`RunnerPs3Error::Refused`], [`RunnerPs3Error::Transport`], an error
/// `may_reclaim` returns, or a [`cleanup`] error from a reclaim.
pub fn preflight<C: ConsoleOps>(
    console: &mut C,
    target: &Target,
    reclaim: bool,
    may_reclaim: &mut dyn FnMut(&Reclaim) -> Result<bool, RunnerPs3Error>,
    clear_with: &str,
    transcript: &mut Transcript,
) -> Result<(), RunnerPs3Error> {
    clear_stale_result(console, target, clear_with, transcript)?;
    if entries(console, GAME_ROOT, transcript)?
        .iter()
        .any(|name| name == target.appid())
    {
        let occupied = || format!("{} already exists on the console", target.game_dir);
        if !reclaim {
            return Err(RunnerPs3Error::Refused {
                reason: occupied(),
                clear_with: format!("{clear_with}, or rerun with --reclaim"),
            });
        }
        let mut contents = Vec::new();
        for name in entries(console, &target.game_dir, transcript)? {
            if name == "USRDIR" {
                for inner in entries(console, &target.usrdir, transcript)? {
                    contents.push(format!("{}/{inner}", target.usrdir));
                }
            }
            contents.push(format!("{}/{name}", target.game_dir));
        }
        let question = Reclaim {
            game_dir: target.game_dir.clone(),
            contents,
        };
        // The operator may take any time to answer.
        console.end_session(transcript);
        if !may_reclaim(&question)? {
            transcript.decision(format!("reclaim of {} declined", target.game_dir));
            return Err(RunnerPs3Error::Refused {
                reason: format!("{}, and the reclaim was declined", occupied()),
                clear_with: clear_with.to_string(),
            });
        }
        transcript.decision(format!("reclaiming {}", target.game_dir));
        cleanup(console, target, transcript)?;
    }
    Ok(())
}

/// Delete a result file that answers `200` once, and refuse when a
/// second check still finds it. Without this step, the wait accepts the
/// result of an earlier run as the result of this run.
///
/// # Errors
///
/// [`RunnerPs3Error::Refused`] naming `clear_with`, and
/// [`RunnerPs3Error::Transport`].
pub fn clear_stale_result<C: ConsoleOps>(
    console: &mut C,
    target: &Target,
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

/// Start the deployed test with one webMAN request,
/// `/play.ps3?<APPID>`, which must answer `200`. It ends the console's
/// session first, because webMAN reloads when the title exits.
///
/// webMAN documents this form as mounting the title under
/// `/dev_hdd0/game/<APPID>` and starting it. The path form
/// (`/play.ps3/<EBOOT path>`) only mounts, and a bare `/play.ps3` starts
/// whatever the XMB then offers, which on the reference console was
/// another installed app.
///
/// # Errors
///
/// [`RunnerPs3Error::Transport`] for a failed request or another status.
pub fn start<C: ConsoleOps>(
    console: &mut C,
    target: &Target,
    transcript: &mut Transcript,
) -> Result<(), RunnerPs3Error> {
    console.end_session(transcript);
    let path = format!("{PLAY_PATH}?{}", target.appid());
    let status = console.http_status(&path, transcript)?;
    if status != 200 {
        return Err(TransportError::UnexpectedStatus { path, status }.into());
    }
    transcript.decision("started");
    Ok(())
}

/// Poll until the start has visibly taken: the status page says the
/// console left the XMB, or the test's result is already there (a test
/// that ran and exited between two polls). At most `timeout_ms / poll_ms`
/// polls, rounded up, each after `sleep(poll_ms)`; a refused poll counts
/// as not yet. A start not taken after [`RESEND_START_AFTER_MS`] is sent
/// once more.
///
/// # Errors
///
/// [`RunnerPs3Error::Usage`] for a zero `poll_ms`, and
/// [`RunnerPs3Error::NotStarted`] when every poll still finds the XMB
/// and no result.
pub fn wait_for_launch<C: ConsoleOps>(
    console: &mut C,
    target: &Target,
    timeout_ms: u64,
    poll_ms: u64,
    sleep: &mut dyn FnMut(Duration),
    transcript: &mut Transcript,
) -> Result<(), RunnerPs3Error> {
    if poll_ms == 0 {
        return Err(RunnerPs3Error::Usage(
            "--poll-ms must be greater than zero".to_string(),
        ));
    }
    let polls = timeout_ms.div_ceil(poll_ms).max(1);
    let resend_at = RESEND_START_AFTER_MS.div_ceil(poll_ms).max(1);
    for poll in 1..=polls {
        sleep(Duration::from_millis(poll_ms));
        if poll == resend_at {
            // A start sent soon after a previous title exits can be ignored
            // while the XMB settles; the console then stays at the XMB.
            transcript.decision(format!(
                "still at the XMB after {poll} poll(s); sending the start once more"
            ));
            start(console, target, transcript)?;
        }
        let left = match console.fetch(STATUS_PATH, transcript) {
            Ok(Some(page)) => at_xmb(&String::from_utf8_lossy(&page)) == Some(false),
            Ok(None) => false,
            Err(error) => {
                transcript.decision(format!("status page unanswered: {error}"));
                false
            }
        };
        if left {
            transcript.decision(format!("left the XMB after {poll} poll(s)"));
            return Ok(());
        }
        if result_written(console, &target.result_path, transcript)? {
            transcript.decision(format!("the result was already there at poll {poll}"));
            return Ok(());
        }
    }
    Err(RunnerPs3Error::NotStarted { timeout_ms })
}

/// Poll the status page until it says the console is back at the XMB:
/// the test exited, so its result file is whole and nothing holds
/// the package open. At most `timeout_ms / poll_ms` polls, rounded up,
/// each after `sleep(poll_ms)`.
///
/// A title's exit reloads the XMB, and webMAN with it, so for a while
/// the console refuses connections; such a poll counts as not yet.
///
/// # Errors
///
/// [`RunnerPs3Error::Usage`] for a zero `poll_ms`, and
/// [`RunnerPs3Error::StillRunning`] when no poll finds the XMB.
pub fn wait_for_xmb<C: ConsoleOps>(
    console: &mut C,
    timeout_ms: u64,
    poll_ms: u64,
    sleep: &mut dyn FnMut(Duration),
    transcript: &mut Transcript,
) -> Result<(), RunnerPs3Error> {
    if poll_ms == 0 {
        return Err(RunnerPs3Error::Usage(
            "--poll-ms must be greater than zero".to_string(),
        ));
    }
    let polls = timeout_ms.div_ceil(poll_ms).max(1);
    for poll in 1..=polls {
        sleep(Duration::from_millis(poll_ms));
        match console.fetch(STATUS_PATH, transcript) {
            Ok(Some(page)) if at_xmb(&String::from_utf8_lossy(&page)) == Some(true) => {
                transcript.decision(format!("back at the XMB after {poll} poll(s)"));
                return Ok(());
            }
            Ok(_) => {}
            Err(error) => transcript.decision(format!("status page unanswered: {error}")),
        }
    }
    Err(RunnerPs3Error::StillRunning { timeout_ms })
}

/// Poll the result path until it answers `200`: at most
/// `timeout_ms / poll_ms` polls, rounded up, each after `sleep(poll_ms)`.
/// The sleep comes before the poll, so the last poll comes at or after
/// the budget and a budget no longer than one interval still waits one
/// interval. The runner reads no clock to wait; the budget is the poll
/// count.
///
/// # Errors
///
/// [`RunnerPs3Error::Usage`] for a zero `poll_ms`,
/// [`RunnerPs3Error::Timeout`] when no poll finds the result, and
/// [`RunnerPs3Error::Transport`] for a failed poll.
pub fn wait_for_result<C: ConsoleOps>(
    console: &mut C,
    target: &Target,
    timeout_ms: u64,
    poll_ms: u64,
    sleep: &mut dyn FnMut(Duration),
    transcript: &mut Transcript,
) -> Result<(), RunnerPs3Error> {
    if poll_ms == 0 {
        return Err(RunnerPs3Error::Usage(
            "--poll-ms must be greater than zero".to_string(),
        ));
    }
    let polls = timeout_ms.div_ceil(poll_ms).max(1);
    for poll in 1..=polls {
        sleep(Duration::from_millis(poll_ms));
        if result_written(console, &target.result_path, transcript)? {
            transcript.decision(format!("result present after {poll} poll(s)"));
            return Ok(());
        }
    }
    Err(RunnerPs3Error::Timeout {
        result_path: target.result_path.clone(),
        timeout_ms,
    })
}

/// The result file's bytes.
///
/// # Errors
///
/// [`RunnerPs3Error::Transport`] for a failed request, and
/// [`RunnerPs3Error::Timeout`] when the file is gone.
pub fn fetch_result<C: ConsoleOps>(
    console: &mut C,
    target: &Target,
    timeout_ms: u64,
    transcript: &mut Transcript,
) -> Result<Vec<u8>, RunnerPs3Error> {
    console
        .fetch(&target.result_path, transcript)?
        .ok_or_else(|| RunnerPs3Error::Timeout {
            result_path: target.result_path.clone(),
            timeout_ms,
        })
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

/// Whether the result file is there for the waits: a file the test is
/// still writing grows between webMAN's header and its body, and counts
/// as not yet.
fn result_written<C: ConsoleOps>(
    console: &mut C,
    path: &str,
    transcript: &mut Transcript,
) -> Result<bool, TransportError> {
    match result_present(console, path, transcript) {
        Err(TransportError::BodyLength { found, expected }) => {
            transcript.decision(format!(
                "{path} grew from {expected} to {found} bytes during the read; not yet"
            ));
            Ok(false)
        }
        other => other,
    }
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

/// The console over webMAN's HTTP and FTP servers.
///
/// Each HTTP request uses its own connection. Every FTP operation shares
/// one logged-in FTP session until [`ConsoleOps::end_session`]. If an
/// exchange fails before its reply arrives, the session is out of step.
/// The console then drops the session, and the next operation logs in
/// again.
///
/// The console does not send a failed command again: a second `MKD` of
/// the lease reads as the lease of another runner. The one exception is
/// a `PASV` without a reply, which webMAN gives under back-to-back runs.
/// `PASV` changes nothing on the console and comes before the transfer
/// command, so the console sends the transfer once more on a new login.
#[derive(Debug)]
pub struct WebmanConsole {
    endpoint: Endpoint,
    ftp: Option<FtpSession<TcpStream>>,
}

impl WebmanConsole {
    /// The console at `endpoint`.
    pub fn new(endpoint: Endpoint) -> Self {
        Self {
            endpoint,
            ftp: None,
        }
    }

    fn connect(&self, address: SocketAddr) -> Result<TcpStream, TransportError> {
        let timeout = Duration::from_millis(self.endpoint.io_timeout_ms);
        let io = |source| TransportError::Io {
            operation: "connect",
            source,
        };
        let stream = TcpStream::connect_timeout(&address, timeout).map_err(|source| {
            TransportError::Unreachable {
                operation: "connect to",
                address: format!("{}:{}", self.endpoint.host, address.port()),
                source,
            }
        })?;
        stream.set_read_timeout(Some(timeout)).map_err(io)?;
        stream.set_write_timeout(Some(timeout)).map_err(io)?;
        Ok(stream)
    }

    fn connect_port(&self, port: u16) -> Result<TcpStream, TransportError> {
        let unresolved = |source| TransportError::Unreachable {
            operation: "resolve",
            address: format!("{}:{port}", self.endpoint.host),
            source,
        };
        let address = (self.endpoint.host.as_str(), port)
            .to_socket_addrs()
            .map_err(unresolved)?
            .next()
            .ok_or_else(|| {
                unresolved(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "the host has no address",
                ))
            })?;
        self.connect(address)
    }

    fn log_in(&self, transcript: &mut Transcript) -> Result<FtpSession<TcpStream>, TransportError> {
        let control = self.connect_port(self.endpoint.ftp_port)?;
        let mut session = FtpSession::open(control, transcript)?;
        session.login_anonymous(transcript)?;
        session.type_binary(transcript)?;
        Ok(session)
    }

    /// Run `exchange` on the FTP session, and log in first if none is open.
    /// The console keeps the session for the next operation, unless the
    /// exchange leaves it out of step.
    fn ftp<T>(
        &mut self,
        transcript: &mut Transcript,
        exchange: impl FnOnce(
            &Self,
            &mut FtpSession<TcpStream>,
            &mut Transcript,
        ) -> Result<T, TransportError>,
    ) -> Result<T, TransportError> {
        let mut session = match self.ftp.take() {
            Some(session) => session,
            None => self.log_in(transcript)?,
        };
        let result = exchange(self, &mut session, transcript);
        if session.in_step() {
            self.ftp = Some(session);
        } else {
            transcript.decision("FTP session dropped: an exchange failed with its reply unread");
        }
        result
    }

    /// Run `transfer` over a new data connection, with the `PASV` retry
    /// that [`WebmanConsole`] describes.
    fn transfer<T>(
        &mut self,
        transcript: &mut Transcript,
        mut transfer: impl FnMut(
            &mut FtpSession<TcpStream>,
            TcpStream,
            &mut Transcript,
        ) -> Result<T, TransportError>,
    ) -> Result<T, TransportError> {
        let mut tries_left = PASV_TRIES;
        loop {
            tries_left -= 1;
            let outcome = self.ftp(transcript, |console, session, transcript| {
                let address = match session.pasv(transcript) {
                    Ok(address) => address,
                    Err(refused) => return Ok(Err(refused)),
                };
                let data = console.connect(SocketAddr::V4(address))?;
                transfer(session, data, transcript).map(Ok)
            })?;
            match outcome {
                Ok(done) => return Ok(done),
                Err(unanswered @ (TransportError::Io { .. } | TransportError::ReplyTruncated))
                    if tries_left > 0 =>
                {
                    transcript.decision(format!(
                    "PASV unanswered ({unanswered}); sending the transfer once more on a new login"
                ))
                }
                Err(refused) => return Err(refused),
            }
        }
    }
}

/// How many times a transfer sends `PASV` before its failure stands.
const PASV_TRIES: u32 = 2;

impl ConsoleOps for WebmanConsole {
    fn http_status(
        &mut self,
        path: &str,
        transcript: &mut Transcript,
    ) -> Result<u16, TransportError> {
        let mut stream = self.connect_port(self.endpoint.http_port)?;
        http::get(&mut stream, &self.endpoint.host, path, transcript).map(|r| r.status)
    }

    fn fetch(
        &mut self,
        path: &str,
        transcript: &mut Transcript,
    ) -> Result<Option<Vec<u8>>, TransportError> {
        let mut stream = self.connect_port(self.endpoint.http_port)?;
        let response = http::get(&mut stream, &self.endpoint.host, path, transcript)?;
        match response.status {
            200 => Ok(Some(response.body)),
            404 => Ok(None),
            status => Err(TransportError::UnexpectedStatus {
                path: path.to_string(),
                status,
            }),
        }
    }

    fn make_dir(&mut self, path: &str, transcript: &mut Transcript) -> Result<(), TransportError> {
        self.ftp(transcript, |_, session, transcript| {
            session.mkd(path, transcript)
        })
    }

    fn store(
        &mut self,
        path: &str,
        bytes: &[u8],
        transcript: &mut Transcript,
    ) -> Result<(), TransportError> {
        self.transfer(transcript, |session, data, transcript| {
            session.stor(path, data, bytes, transcript)
        })
    }

    fn list(
        &mut self,
        dir: &str,
        transcript: &mut Transcript,
    ) -> Result<Vec<String>, TransportError> {
        self.transfer(transcript, |session, data, transcript| {
            session.nlst(dir, data, transcript)
        })
    }

    fn delete(&mut self, path: &str, transcript: &mut Transcript) -> Result<(), TransportError> {
        self.ftp(transcript, |_, session, transcript| {
            session.dele(path, transcript)
        })
    }

    fn remove_dir(
        &mut self,
        path: &str,
        transcript: &mut Transcript,
    ) -> Result<(), TransportError> {
        self.ftp(transcript, |_, session, transcript| {
            session.rmd(path, transcript)
        })
    }

    fn end_session(&mut self, transcript: &mut Transcript) {
        if let Some(session) = self.ftp.take() {
            if let Err(error) = session.quit(transcript) {
                transcript.decision(format!(
                    "FTP session closed without its QUIT reply: {error}"
                ));
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/run_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/run_session_tests.rs"]
mod run_session_tests;

#[cfg(test)]
#[path = "tests/webman_session_tests.rs"]
mod webman_session_tests;
