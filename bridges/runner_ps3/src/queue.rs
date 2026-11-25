//! One host-wide queue in front of the reference console.
//!
//! Every verb that talks to the console waits its turn here first, so
//! two runners on one machine never use the console at once. The queue
//! is a directory of tickets, [`host_wide_dir`] unless a test names
//! another. Ticket `n` is two files:
//!
//! - `<n>.ticket`, which its owner holds an exclusive OS file lock on
//!   while it waits and while it uses the console;
//! - `<n>.holder`, the owner's process, user and command line as text.
//!
//! The live ticket with the lowest number holds the console. A new
//! ticket takes the number above every ticket in the directory, so the
//! queue serves its waiters first in, first out. The OS releases a file
//! lock when its process ends, however it ends. So a ticket whose lock
//! another process can take has no live owner, and the next scan
//! removes it.
//!
//! The directory also holds these files:
//!
//! - `guard`, which a scan locks, so that no ticket is taken or removed
//!   during the scan;
//! - `down`, which a runner writes when the console did not answer it;
//!   a waiter that finds it stops waiting;
//! - `log`, with one line for each event.
//!
//! The queue serves the processes of one account. No file holds the
//! console's address or a value that identifies the console.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::transcript;

/// The longest a waiter waits for its turn, unless a test names another.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(30 * 60);

/// The wait between two looks at the queue, unless a test names another.
pub const DEFAULT_POLL: Duration = Duration::from_secs(1);

/// The tries to lock `guard` before a scan gives up.
const GUARD_TRIES: u32 = 5_000;

/// The wait between two tries to lock `guard`.
const GUARD_PAUSE: Duration = Duration::from_millis(1);

/// The size above which `log` moves to `log.old` before the next line.
const LOG_LIMIT: u64 = 1 << 20;

/// The file whose lock a scan holds.
const GUARD_FILE: &str = "guard";

/// The file that says the console did not answer.
pub const DOWN_FILE: &str = "down";

/// The event log.
pub const LOG_FILE: &str = "log";

/// The queue's directory on this machine.
///
/// It is `%ProgramData%\cellgov\console_queue` on Windows and
/// `/tmp/cellgov-console-queue` elsewhere. Both are one directory for
/// the whole machine. The runner does not use the temp directory: each
/// session or sandbox can set `TMP`, `TEMP` or `TMPDIR` differently, and
/// then two runners see two queues.
pub fn host_wide_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("ProgramData")
            .filter(|value| !value.is_empty())
            .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from)
            .join("cellgov")
            .join("console_queue")
    } else {
        PathBuf::from("/tmp/cellgov-console-queue")
    }
}

/// What a holder file says: the process, the user and machine, and the
/// command line, which the caller masks first.
pub fn holder_text(who: &str, command_line: &str) -> String {
    format!("pid {}, {who}, running {command_line}", std::process::id())
}

/// `words` joined as a shell line, with the console's `host` masked
/// wherever it appears and every console identifier the transcript
/// redactor knows masked.
pub fn masked_command_line(words: &[String], host: &str) -> String {
    const MASK: &str = "<host>";
    let host = host.trim();
    let mut previous = "";
    let mut line = Vec::with_capacity(words.len());
    for word in words {
        let masked = if previous == "--host" || (!host.is_empty() && word == host) {
            MASK.to_string()
        } else if word.starts_with("--host=") {
            format!("--host={MASK}")
        } else if !host.is_empty() {
            word.replace(host, MASK)
        } else {
            word.clone()
        };
        previous = word;
        line.push(
            if masked.is_empty() || masked.contains(char::is_whitespace) {
                format!("\"{masked}\"")
            } else {
                masked
            },
        );
    }
    transcript::redact(&line.join(" "))
}

/// Why a waiter did not get its turn.
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    /// The wait ran out before the waiter's turn came.
    #[error(
        "waited {waited_ms} ms in the console queue at {} with {ahead} ahead, behind {holder}; \
         run again later",
        dir.display()
    )]
    Timeout {
        /// The queue's directory, whose holder files name every waiter.
        dir: PathBuf,
        /// How long the waiter waited.
        waited_ms: u64,
        /// The live tickets ahead of the waiter when the wait ran out.
        ahead: usize,
        /// What the holder file of the ticket that holds the console says.
        holder: String,
    },
    /// The console did not answer the runner that held it.
    #[error("console down; the operator must restart it ({cause})")]
    ConsoleDown {
        /// What did not answer, with no value that identifies the console.
        cause: String,
    },
    /// A file of the queue did not read, write or lock.
    #[error("console queue {}: {source}", path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Another process kept `guard` locked for every try.
    #[error("console queue {}: another runner kept it locked for {tries} tries", path.display())]
    GuardBusy {
        /// The guard file.
        path: PathBuf,
        /// The tries made.
        tries: u32,
    },
}

fn io_at(path: &Path) -> impl FnOnce(std::io::Error) -> QueueError + '_ {
    move |source| QueueError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The queue: where it lives, how long a waiter waits, and how often it
/// looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    dir: PathBuf,
    wait: Duration,
    poll: Duration,
}

/// Where a ticket stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Position {
    /// No live ticket is ahead: the ticket holds the console.
    Head,
    /// `ahead` live tickets are ahead; `holder` names the first.
    Behind { ahead: usize, holder: String },
}

impl Queue {
    /// The queue in `dir`. A waiter looks every `poll` and gives up after
    /// `wait`. A `poll` under one millisecond counts as one millisecond.
    pub fn new(dir: PathBuf, wait: Duration, poll: Duration) -> Self {
        Self {
            dir,
            wait,
            poll: poll.max(Duration::from_millis(1)),
        }
    }

    /// The queue in [`host_wide_dir`], with [`DEFAULT_WAIT`] and
    /// [`DEFAULT_POLL`].
    pub fn host_wide() -> Self {
        Self::new(host_wide_dir(), DEFAULT_WAIT, DEFAULT_POLL)
    }

    /// The queue's directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Take a ticket for `holder` and wait for its turn. `sleep` waits
    /// between two looks at the queue.
    ///
    /// # Errors
    ///
    /// - [`QueueError::Timeout`] when `wait` runs out first;
    /// - [`QueueError::ConsoleDown`] when a runner ahead marked the
    ///   console down;
    /// - [`QueueError::Io`] and [`QueueError::GuardBusy`] when the
    ///   queue's files do not serve.
    ///
    /// The ticket leaves the queue on every error.
    pub fn join(&self, holder: &str, sleep: &mut dyn FnMut(Duration)) -> Result<Turn, QueueError> {
        let ticket = self.enqueue(holder)?;
        let polls = self.wait.as_millis().div_ceil(self.poll.as_millis());
        let mut slept: u128 = 0;
        loop {
            let (ahead, head) = match self.position(&ticket)? {
                Position::Head => {
                    self.log(&format!("ticket {} granted", ticket.number));
                    return Ok(Turn { ticket });
                }
                Position::Behind { ahead, holder } => (ahead, holder),
            };
            if let Some(cause) = self.down()? {
                self.log(&format!("ticket {} left: console down", ticket.number));
                return Err(QueueError::ConsoleDown { cause });
            }
            if slept >= polls {
                let waited_ms = u64::try_from(slept * self.poll.as_millis()).unwrap_or(u64::MAX);
                self.log(&format!(
                    "ticket {} left: waited {waited_ms} ms",
                    ticket.number
                ));
                return Err(QueueError::Timeout {
                    dir: self.dir.clone(),
                    waited_ms,
                    ahead,
                    holder: head,
                });
            }
            sleep(self.poll);
            slept += 1;
        }
    }

    /// What `down` says, or `None` when the console is not marked down.
    ///
    /// # Errors
    ///
    /// [`QueueError::Io`] when the file exists and does not read.
    pub fn down(&self) -> Result<Option<String>, QueueError> {
        let path = self.dir.join(DOWN_FILE);
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text.trim().to_string())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_at(&path)(error)),
        }
    }

    /// Take the next ticket for `holder`, locked and named.
    pub(crate) fn enqueue(&self, holder: &str) -> Result<Ticket, QueueError> {
        fs::create_dir_all(&self.dir).map_err(io_at(&self.dir))?;
        let _guard = Guard::take(&self.dir)?;
        let number = self.numbers()?.last().map_or(1, |last| last + 1);
        let mut ticket = Ticket {
            number,
            dir: self.dir.clone(),
            lock: None,
        };
        let path = ticket.ticket_path();
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(io_at(&path))?;
        file.try_lock()
            .map_err(|refused| io_at(&path)(std::io::Error::from(refused)))?;
        ticket.lock = Some(file);
        let holder_path = ticket.holder_path();
        fs::write(&holder_path, holder).map_err(io_at(&holder_path))?;
        self.log(&format!("ticket {number} joined: {holder}"));
        Ok(ticket)
    }

    /// Where `ticket` stands, after removing every ticket ahead of it
    /// that has no live owner.
    pub(crate) fn position(&self, ticket: &Ticket) -> Result<Position, QueueError> {
        let _guard = Guard::take(&self.dir)?;
        let mut ahead = Vec::new();
        for number in self.numbers()? {
            if number >= ticket.number {
                break;
            }
            if self.alive(number)? {
                ahead.push(number);
            } else {
                self.reclaim(number);
            }
        }
        Ok(match ahead.first() {
            None => Position::Head,
            Some(&first) => Position::Behind {
                ahead: ahead.len(),
                holder: self.holder_of(first),
            },
        })
    }

    /// Every ticket number in the directory, ascending.
    fn numbers(&self) -> Result<Vec<u64>, QueueError> {
        let mut numbers = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(io_at(&self.dir))? {
            let name = entry.map_err(io_at(&self.dir))?.file_name();
            if let Some(number) = name
                .to_str()
                .and_then(|name| name.strip_suffix(".ticket"))
                .and_then(|stem| stem.parse::<u64>().ok())
            {
                numbers.push(number);
            }
        }
        numbers.sort_unstable();
        Ok(numbers)
    }

    /// Whether a live process holds ticket `number`. A ticket that no
    /// longer exists has no owner. A ticket this process may not open
    /// counts as live, so the scan never removes a ticket it cannot
    /// test.
    fn alive(&self, number: u64) -> Result<bool, QueueError> {
        let path = ticket_path(&self.dir, number);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return Ok(true),
            Err(error) => return Err(io_at(&path)(error)),
        };
        match file.try_lock() {
            Ok(()) => Ok(false),
            Err(TryLockError::WouldBlock) => Ok(true),
            Err(TryLockError::Error(error)) => Err(io_at(&path)(error)),
        }
    }

    /// Remove ticket `number`, whose owner is gone. A file that does not
    /// remove stays, unlocked, and the scan treats it as gone.
    fn reclaim(&self, number: u64) {
        let holder = self.holder_of(number);
        remove_quietly(&ticket_path(&self.dir, number));
        remove_quietly(&holder_path(&self.dir, number));
        self.log(&format!(
            "ticket {number} reclaimed: no live owner ({holder})"
        ));
    }

    /// What ticket `number`'s holder file says.
    fn holder_of(&self, number: u64) -> String {
        fs::read_to_string(holder_path(&self.dir, number)).map_or_else(
            |_| format!("ticket {number}, whose holder file does not read"),
            |text| text.trim().to_string(),
        )
    }

    /// Add `event` to the log. The log is a diagnostic, so a line that
    /// does not store never refuses the console.
    fn log(&self, event: &str) {
        let path = self.dir.join(LOG_FILE);
        if fs::metadata(&path).is_ok_and(|meta| meta.len() > LOG_LIMIT) {
            let _moved = fs::rename(&path, self.dir.join("log.old"));
        }
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _stored = writeln!(file, "{seconds} {event}");
        }
    }
}

fn ticket_path(dir: &Path, number: u64) -> PathBuf {
    dir.join(format!("{number:020}.ticket"))
}

fn holder_path(dir: &Path, number: u64) -> PathBuf {
    dir.join(format!("{number:020}.holder"))
}

/// Remove `path`. A file that does not remove stays, and the next scan
/// finds it unlocked.
fn remove_quietly(path: &Path) {
    let _removed = fs::remove_file(path);
}

/// A held lock on `guard`, released when it drops.
struct Guard {
    _file: File,
}

impl Guard {
    fn take(dir: &Path) -> Result<Self, QueueError> {
        let path = dir.join(GUARD_FILE);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(io_at(&path))?;
        for _ in 0..GUARD_TRIES {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock) => std::thread::sleep(GUARD_PAUSE),
                Err(TryLockError::Error(error)) => return Err(io_at(&path)(error)),
            }
        }
        Err(QueueError::GuardBusy {
            path,
            tries: GUARD_TRIES,
        })
    }
}

/// A ticket this process holds. Dropping it unlocks it and removes its
/// files, so a waiter leaves the queue on every path out of a verb.
#[derive(Debug)]
pub(crate) struct Ticket {
    number: u64,
    dir: PathBuf,
    lock: Option<File>,
}

impl Ticket {
    /// The ticket's number.
    #[cfg(test)]
    pub(crate) fn number(&self) -> u64 {
        self.number
    }

    fn ticket_path(&self) -> PathBuf {
        ticket_path(&self.dir, self.number)
    }

    fn holder_path(&self) -> PathBuf {
        holder_path(&self.dir, self.number)
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        // Remove both files while this ticket holds its lock, then unlock.
        // While the lock holds, no scan reclaims the number and no joiner
        // reuses it, so these removals never hit a later ticket's files.
        // On Windows, std opens files with delete sharing, so the removal
        // succeeds with the handle open. A name that stays listed until
        // the unlock does not open, and the scan counts it as live.
        remove_quietly(&self.holder_path());
        remove_quietly(&self.ticket_path());
        drop(self.lock.take());
    }
}

/// The console, held: the ticket at the head of the queue. Dropping the
/// turn gives the console to the next waiter.
#[derive(Debug)]
#[must_use = "a turn holds the console until it drops"]
pub struct Turn {
    ticket: Ticket,
}

impl Turn {
    /// Clear the down mark: the console answered this runner.
    ///
    /// # Errors
    ///
    /// [`QueueError::Io`] when the mark exists and does not remove.
    pub fn console_up(&self) -> Result<(), QueueError> {
        let path = self.ticket.dir.join(DOWN_FILE);
        match fs::remove_file(&path) {
            Ok(()) => {
                self.queue().log(&format!(
                    "ticket {}: the console answered; down mark cleared",
                    self.ticket.number
                ));
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_at(&path)(error)),
        }
    }

    /// Mark the console down for every waiter, give up the turn, and
    /// return the error the runner reports. `cause` holds no value that
    /// identifies the console.
    pub fn console_down(self, cause: &str) -> QueueError {
        let path = self.ticket.dir.join(DOWN_FILE);
        let cause = match fs::write(&path, cause) {
            Ok(()) => cause.to_string(),
            Err(error) => format!("{cause}; the down mark did not store: {error}"),
        };
        self.queue().log(&format!(
            "ticket {} marked the console down: {cause}",
            self.ticket.number
        ));
        QueueError::ConsoleDown { cause }
    }

    /// The queue the turn belongs to, for its log.
    fn queue(&self) -> Queue {
        Queue::new(self.ticket.dir.clone(), Duration::ZERO, Duration::ZERO)
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        self.queue()
            .log(&format!("ticket {} released", self.ticket.number));
    }
}

#[cfg(test)]
#[path = "tests/queue_tests.rs"]
mod tests;
