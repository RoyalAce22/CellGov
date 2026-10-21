//! The FTP session `WebmanConsole` keeps between operations, against a
//! scripted server on a loopback port: which commands reach the server,
//! on which connection.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::{ConsoleOps, WebmanConsole};
use crate::transcript::Transcript;
use crate::transport::{Endpoint, TransportError};

/// The path whose `DELE` the server refuses with `550`.
const MISSING: &str = "/dev_hdd0/tmp/missing";

/// The path whose `RMD` the server never answers: it closes the
/// connection instead.
const VANISHING: &str = "/dev_hdd0/tmp/vanishing";

/// The path whose `STOR` the server takes in full and then closes the
/// connection without its closing reply.
const DROPPED: &str = "/dev_hdd0/tmp/dropped";

/// The most commands the server reads on one connection.
const MAX_COMMANDS: usize = 32;

/// The most connections the server accepts.
const MAX_CONNECTIONS: usize = 8;

/// The most 5 ms waits the server makes for a connection.
const MAX_IDLE_POLLS: usize = 2_000;

/// A scripted FTP server, one connection at a time.
struct Server {
    port: u16,
    done: mpsc::Sender<()>,
    heard: thread::JoinHandle<Vec<Vec<String>>>,
}

impl Server {
    fn start() -> Self {
        Self::leaving_pasvs_unanswered(0)
    }

    /// A server that gives no reply to its first `unanswered` `PASV`
    /// commands and keeps the connection open, as webMAN does under
    /// back-to-back runs.
    fn leaving_pasvs_unanswered(mut unanswered: usize) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("a loopback port");
        let port = listener.local_addr().expect("bound").port();
        listener.set_nonblocking(true).expect("non-blocking accept");
        let (done, finished) = mpsc::channel::<()>();
        let heard = thread::spawn(move || {
            let mut connections = Vec::new();
            let mut idle = 0;
            while connections.len() < MAX_CONNECTIONS && idle < MAX_IDLE_POLLS {
                match listener.accept() {
                    Ok((stream, _)) => {
                        idle = 0;
                        connections.push(answer(stream, &mut unanswered));
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        if finished.try_recv().is_ok() {
                            break;
                        }
                        idle += 1;
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            }
            connections
        });
        Self { port, done, heard }
    }

    fn console(&self) -> WebmanConsole {
        self.console_waiting(5_000)
    }

    /// A console that waits `io_timeout_ms` for each reply.
    fn console_waiting(&self, io_timeout_ms: u64) -> WebmanConsole {
        WebmanConsole::new(Endpoint {
            ftp_port: self.port,
            io_timeout_ms,
            ..Endpoint::new("127.0.0.1")
        })
    }

    /// The commands each connection sent, in connection order.
    fn finish(self) -> Vec<Vec<String>> {
        self.done.send(()).expect("server still running");
        self.heard.join().expect("server thread")
    }
}

/// Serve one control connection; returns the commands it sent.
/// `unanswered` counts down the `PASV` commands still to leave unanswered.
fn answer(stream: TcpStream, unanswered: &mut usize) -> Vec<String> {
    // An accepted socket can inherit the listener's non-blocking mode.
    stream.set_nonblocking(false).expect("blocking reads");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let mut out = stream.try_clone().expect("writer");
    let mut reply = |line: &str| {
        out.write_all(format!("{line}\r\n").as_bytes())
            .expect("reply");
    };
    let mut reader = BufReader::new(stream);
    let mut heard = Vec::new();
    let mut passive: Option<TcpListener> = None;
    reply("220 scripted");
    for _ in 0..MAX_COMMANDS {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let command = line.trim_end().to_string();
        heard.push(command.clone());
        let (verb, argument) = command.split_once(' ').unwrap_or((&command, ""));
        match (verb, argument) {
            ("USER", _) => reply("331 password"),
            ("PASS", _) => reply("230 in"),
            ("TYPE", "I") => reply("200 binary"),
            ("MKD", path) => reply(&format!("257 \"{path}\" made")),
            ("DELE", MISSING) => reply("550 no such file"),
            ("RMD", VANISHING) => break,
            ("DELE" | "RMD", _) => reply("250 done"),
            ("PASV", _) if *unanswered > 0 => *unanswered -= 1,
            ("PASV", _) => {
                let data = TcpListener::bind(("127.0.0.1", 0)).expect("a data port");
                let port = data.local_addr().expect("bound").port();
                reply(&format!(
                    "227 Entering Passive Mode (127,0,0,1,{},{})",
                    port >> 8,
                    port & 0xff
                ));
                passive = Some(data);
            }
            ("NLST", _) => match passive.take() {
                Some(data) => {
                    reply("150 listing");
                    let (mut names, _) = data.accept().expect("the client's data connection");
                    names.write_all(b"a\r\nb\r\n").expect("names");
                    drop(names);
                    reply("226 listed");
                }
                None => reply("425 no PASV"),
            },
            ("STOR", DROPPED) => match passive.take() {
                Some(data) => {
                    reply("150 receiving");
                    let (mut file, _) = data.accept().expect("the client's data connection");
                    file.set_read_timeout(Some(Duration::from_secs(5)))
                        .expect("read timeout");
                    let mut received = Vec::new();
                    file.read_to_end(&mut received).expect("the file");
                    break;
                }
                None => reply("425 no PASV"),
            },
            ("QUIT", _) => {
                reply("221 bye");
                break;
            }
            _ => reply("502 not scripted"),
        }
    }
    heard
}

fn login() -> Vec<String> {
    ["USER anonymous", "PASS anonymous@", "TYPE I"]
        .map(str::to_string)
        .to_vec()
}

fn session(commands: &[&str]) -> Vec<String> {
    let mut sent = login();
    sent.extend(commands.iter().map(|c| (*c).to_string()));
    sent
}

#[test]
fn ftp_operations_share_one_login_until_the_session_ends() {
    let server = Server::start();
    let mut console = server.console();
    let mut transcript = Transcript::new();
    console
        .make_dir("/dev_hdd0/tmp/a", &mut transcript)
        .expect("MKD");
    console
        .delete("/dev_hdd0/tmp/b", &mut transcript)
        .expect("DELE");
    console
        .remove_dir("/dev_hdd0/tmp/c", &mut transcript)
        .expect("RMD");
    console.end_session(&mut transcript);
    console
        .make_dir("/dev_hdd0/tmp/d", &mut transcript)
        .expect("MKD");
    console.end_session(&mut transcript);
    assert_eq!(
        server.finish(),
        [
            session(&[
                "MKD /dev_hdd0/tmp/a",
                "DELE /dev_hdd0/tmp/b",
                "RMD /dev_hdd0/tmp/c",
                "QUIT"
            ]),
            session(&["MKD /dev_hdd0/tmp/d", "QUIT"]),
        ]
    );
}

#[test]
fn a_refused_command_keeps_the_session() {
    let server = Server::start();
    let mut console = server.console();
    let mut transcript = Transcript::new();
    let refused = console
        .delete(MISSING, &mut transcript)
        .expect_err("the server refuses it");
    assert!(
        matches!(refused, TransportError::UnexpectedReply { code: 550, .. }),
        "{refused:?}"
    );
    console
        .make_dir("/dev_hdd0/tmp/a", &mut transcript)
        .expect("MKD");
    console.end_session(&mut transcript);
    assert_eq!(
        server.finish(),
        [session(&[
            &format!("DELE {MISSING}"),
            "MKD /dev_hdd0/tmp/a",
            "QUIT"
        ])]
    );
}

#[test]
fn an_exchange_left_out_of_step_logs_in_again_without_resending_it() {
    let server = Server::start();
    let mut console = server.console();
    let mut transcript = Transcript::new();
    let lost = console
        .remove_dir(VANISHING, &mut transcript)
        .expect_err("the server closes without a reply");
    assert!(matches!(lost, TransportError::ReplyTruncated), "{lost:?}");
    console
        .make_dir("/dev_hdd0/tmp/a", &mut transcript)
        .expect("MKD");
    console.end_session(&mut transcript);
    assert_eq!(
        server.finish(),
        [
            session(&[&format!("RMD {VANISHING}")]),
            session(&["MKD /dev_hdd0/tmp/a", "QUIT"]),
        ]
    );
    assert!(
        transcript.lines().iter().any(|line| line
            .ends_with("= FTP session dropped: an exchange failed with its reply unread")),
        "{:#?}",
        transcript.lines()
    );
}

#[test]
fn ending_a_session_that_is_not_open_sends_nothing() {
    let server = Server::start();
    let mut console = server.console();
    let mut transcript = Transcript::new();
    console.end_session(&mut transcript);
    assert!(server.finish().is_empty());
    assert!(transcript.lines().is_empty(), "{:#?}", transcript.lines());
}

/// The number of retry lines in a transcript.
fn retries(transcript: &Transcript) -> usize {
    transcript
        .lines()
        .iter()
        .filter(|line| line.contains("= PASV unanswered"))
        .count()
}

#[test]
fn an_unanswered_pasv_is_sent_once_more_on_a_new_login() {
    let server = Server::leaving_pasvs_unanswered(1);
    let mut console = server.console_waiting(500);
    let mut transcript = Transcript::new();
    let names = console
        .list("/dev_hdd0/game", &mut transcript)
        .expect("the second PASV is answered");
    console.end_session(&mut transcript);
    assert_eq!(names, ["a", "b"]);
    assert_eq!(
        server.finish(),
        [
            session(&["PASV"]),
            session(&["PASV", "NLST /dev_hdd0/game", "QUIT"]),
        ]
    );
    assert_eq!(retries(&transcript), 1, "{:#?}", transcript.lines());
}

#[test]
fn a_second_unanswered_pasv_fails_the_transfer() {
    let server = Server::leaving_pasvs_unanswered(2);
    let mut console = server.console_waiting(500);
    let mut transcript = Transcript::new();
    let error = console
        .list("/dev_hdd0/game", &mut transcript)
        .expect_err("both PASV commands go unanswered");
    console.end_session(&mut transcript);
    assert!(matches!(error, TransportError::Io { .. }), "{error:?}");
    assert_eq!(server.finish(), [session(&["PASV"]), session(&["PASV"])]);
    assert_eq!(retries(&transcript), 1, "{:#?}", transcript.lines());
}

#[test]
fn a_store_whose_closing_reply_is_lost_is_not_sent_again() {
    let server = Server::start();
    let mut console = server.console();
    let mut transcript = Transcript::new();
    let lost = console
        .store(DROPPED, b"stale", &mut transcript)
        .expect_err("the server closes without the closing reply");
    console.end_session(&mut transcript);
    // An unanswered PASV gives the same error kind; here the console
    // already sent the STOR.
    assert!(matches!(lost, TransportError::ReplyTruncated), "{lost:?}");
    assert_eq!(
        server.finish(),
        [session(&["PASV", &format!("STOR {DROPPED}")])]
    );
    assert_eq!(retries(&transcript), 0, "{:#?}", transcript.lines());
}
