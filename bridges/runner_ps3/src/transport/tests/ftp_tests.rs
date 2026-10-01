//! The FTP client over in-memory streams: the reply parser with
//! single- and multi-line replies, the PASV tuple, and a scripted
//! session from greeting to quit with its transcript.

use std::io::{Cursor, Read, Write};

use super::*;

struct MemoryWire {
    input: Cursor<Vec<u8>>,
    output: Vec<u8>,
    write_closed: bool,
}

impl MemoryWire {
    fn answering(script: &str) -> Self {
        Self {
            input: Cursor::new(script.as_bytes().to_vec()),
            output: Vec::new(),
            write_closed: false,
        }
    }

    fn sent(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }
}

impl Read for MemoryWire {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.input.read(buf)
    }
}

impl Write for MemoryWire {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.output.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl DataConnection for MemoryWire {
    fn close_write(&mut self) -> std::io::Result<()> {
        self.write_closed = true;
        Ok(())
    }
}

/// A data connection whose every write fails.
struct BrokenData;

impl Read for BrokenData {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::ConnectionReset.into())
    }
}

impl Write for BrokenData {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::ConnectionReset.into())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl DataConnection for BrokenData {
    fn close_write(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_single_line_reply_parses_with_the_bytes_it_spans() {
    let (reply, used) = parse_reply("226 Transfer complete\r\n150 next\r\n")
        .expect("parses")
        .expect("complete");
    assert_eq!(reply.code, 226);
    assert_eq!(reply.lines, ["Transfer complete"]);
    assert_eq!(used, "226 Transfer complete\r\n".len());
}

#[test]
fn a_multi_line_reply_runs_to_the_line_that_closes_its_code() {
    let text = "211-Features:\r\n MDTM\r\n 211-not the end\r\n211 End\r\n";
    let (reply, used) = parse_reply(text).expect("parses").expect("complete");
    assert_eq!(reply.code, 211);
    assert_eq!(reply.lines, ["Features:", "MDTM", "211-not the end", "End"]);
    assert_eq!(used, text.len());
}

#[test]
fn a_reply_the_text_has_not_finished_is_incomplete() {
    for text in [
        "",
        "226 Transfer",
        "211-Features:\r\n MDTM\r\n",
        "211-a\r\n211 b",
    ] {
        assert_eq!(parse_reply(text).expect("no error"), None, "{text:?}");
    }
}

#[test]
fn a_line_that_does_not_open_with_a_code_is_refused() {
    for line in ["hello\r\n", "22 short\r\n", "226xTransfer\r\n"] {
        assert!(
            matches!(parse_reply(line), Err(TransportError::BadReplyLine(_))),
            "{line:?}"
        );
    }
}

#[test]
fn reading_a_reply_consumes_that_reply_and_nothing_after_it() {
    let mut wire = MemoryWire::answering("220-webMAN\r\n220 ready\r\n331 password\r\n");
    let first = read_reply(&mut wire).expect("first");
    assert_eq!(
        (first.code, first.text()),
        (220, "webMAN / ready".to_string())
    );
    let second = read_reply(&mut wire).expect("second");
    assert_eq!(second.code, 331);
    assert!(matches!(
        read_reply(&mut wire),
        Err(TransportError::ReplyTruncated)
    ));
}

/// A stream that interrupts every other read before it yields a byte.
struct InterruptingWire {
    input: Cursor<Vec<u8>>,
    interrupt_next: bool,
}

impl Read for InterruptingWire {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.interrupt_next = !self.interrupt_next;
        if self.interrupt_next {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        self.input.read(buf)
    }
}

#[test]
fn an_interrupted_read_is_retried_not_reported() {
    let mut wire = InterruptingWire {
        input: Cursor::new(b"220-webMAN\r\n220 ready\r\n".to_vec()),
        interrupt_next: false,
    };
    let reply = read_reply(&mut wire).expect("interruptions are retried");
    assert_eq!(reply.code, 220);
    assert!(matches!(
        read_reply(&mut wire),
        Err(TransportError::ReplyTruncated)
    ));
}

#[test]
fn the_pasv_tuple_is_the_data_address() {
    let address = parse_pasv("Entering Passive Mode (10,77,0,2,195,80).").expect("tuple");
    assert_eq!(address.to_string(), "10.77.0.2:50000");
    for text in [
        "Entering Passive Mode",
        "(10,77,0,2,195)",
        "(10,77,0,2,195,256)",
        "(a,b,c,d,e,f)",
    ] {
        assert!(
            matches!(parse_pasv(text), Err(TransportError::BadPasv(_))),
            "{text:?}"
        );
    }
}

#[test]
fn a_session_logs_in_stores_a_file_and_quits_with_every_exchange_transcribed() {
    let mut control = MemoryWire::answering(concat!(
        "220 webMAN ready\r\n",
        "331 password\r\n",
        "230 logged in\r\n",
        "200 binary\r\n",
        "227 Entering Passive Mode (10,77,0,2,195,80)\r\n",
        "150 opening\r\n",
        "226 Transfer complete\r\n",
        "221 bye\r\n",
    ));
    let mut data = MemoryWire::answering("");
    let mut transcript = Transcript::new();
    let mut session = FtpSession::open(&mut control, &mut transcript).expect("greeting");
    session.login_anonymous(&mut transcript).expect("login");
    session.type_binary(&mut transcript).expect("binary");
    let address = session.pasv(&mut transcript).expect("pasv");
    assert_eq!(address.port(), 50000);
    session
        .stor("/dev_hdd0/tmp/x.bin", &mut data, b"CGOV", &mut transcript)
        .expect("stor");
    session.quit(&mut transcript).expect("quit");

    assert_eq!(data.output, b"CGOV");
    assert!(data.write_closed, "a borrowed data stream still closes");
    assert_eq!(
        control.sent(),
        "USER anonymous\r\nPASS anonymous@\r\nTYPE I\r\nPASV\r\nSTOR /dev_hdd0/tmp/x.bin\r\nQUIT\r\n"
    );
    assert_eq!(
        transcript.lines(),
        [
            "#0001 < 220 webMAN ready",
            "#0002 > USER anonymous",
            "#0003 < 331 password",
            "#0004 > PASS anonymous@",
            "#0005 < 230 logged in",
            "#0006 > TYPE I",
            "#0007 < 200 binary",
            "#0008 > PASV",
            "#0009 < 227 Entering Passive Mode (10,77,0,2,195,80)",
            "#0010 > STOR /dev_hdd0/tmp/x.bin (4 bytes)",
            "#0011 < 150 opening",
            "#0012 < 226 Transfer complete",
            "#0013 > QUIT",
            "#0014 < 221 bye",
        ]
    );
}

#[test]
fn a_listing_reads_the_names_over_the_data_connection() {
    let mut control = MemoryWire::answering("220 ready\r\n150 here\r\n226 done\r\n");
    let mut data = MemoryWire::answering("EBOOT.BIN\r\nPARAM.SFO\r\n\r\nspu_main.elf\n");
    let mut transcript = Transcript::new();
    let mut session = FtpSession::open(&mut control, &mut transcript).expect("greeting");
    let names = session
        .nlst(
            "/dev_hdd0/game/CGOV00001/USRDIR",
            &mut data,
            &mut transcript,
        )
        .expect("nlst");
    assert_eq!(names, ["EBOOT.BIN", "PARAM.SFO", "spu_main.elf"]);
}

#[test]
fn a_reply_the_command_does_not_accept_names_the_command_and_the_code() {
    let mut control = MemoryWire::answering("220 ready\r\n550 No such file\r\n");
    let mut transcript = Transcript::new();
    let mut session = FtpSession::open(&mut control, &mut transcript).expect("greeting");
    let err = session
        .dele("/dev_hdd0/tmp/cgov_x.bin", &mut transcript)
        .expect_err("550");
    match err {
        TransportError::UnexpectedReply {
            command,
            code,
            text,
        } => {
            assert_eq!(command, "DELE /dev_hdd0/tmp/cgov_x.bin");
            assert_eq!(code, 550);
            assert_eq!(text, "No such file");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_data_failure_leaves_the_session_refusing_every_later_command() {
    let mut control = MemoryWire::answering(concat!(
        "220 ready\r\n",
        "150 opening\r\n",
        "426 transfer aborted\r\n",
        "250 deleted\r\n",
    ));
    let mut transcript = Transcript::new();
    {
        let mut session = FtpSession::open(&mut control, &mut transcript).expect("greeting");
        let stored = session.stor("/dev_hdd0/tmp/x.bin", BrokenData, b"CGOV", &mut transcript);
        assert!(
            matches!(
                stored,
                Err(TransportError::Io {
                    operation: "FTP data write",
                    ..
                })
            ),
            "{stored:?}"
        );
        match session.dele("/dev_hdd0/tmp/x.bin", &mut transcript) {
            Err(TransportError::Desynced(command)) => {
                assert_eq!(command, "DELE /dev_hdd0/tmp/x.bin");
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(
        control.sent(),
        "STOR /dev_hdd0/tmp/x.bin\r\n",
        "nothing after the failure"
    );
}

#[test]
fn a_reply_the_command_refuses_still_ends_its_exchange() {
    let mut control = MemoryWire::answering("220 ready\r\n550 No such file\r\n250 removed\r\n");
    let mut transcript = Transcript::new();
    let mut session = FtpSession::open(&mut control, &mut transcript).expect("greeting");
    session
        .dele("/dev_hdd0/tmp/cgov_x.bin", &mut transcript)
        .expect_err("550");
    session
        .rmd("/dev_hdd0/game/CGOV00001", &mut transcript)
        .expect("the session is still in step");
}

#[test]
fn a_preliminary_reply_the_command_refuses_leaves_its_closing_reply_owed() {
    let mut control = MemoryWire::answering("220 ready\r\n150 opening\r\n226 done\r\n");
    let mut transcript = Transcript::new();
    {
        let mut session = FtpSession::open(&mut control, &mut transcript).expect("greeting");
        session
            .dele("/dev_hdd0/tmp/cgov_x.bin", &mut transcript)
            .expect_err("150 is not 250");
        match session.rmd("/dev_hdd0/game/CGOV00001", &mut transcript) {
            Err(TransportError::Desynced(command)) => {
                assert_eq!(command, "RMD /dev_hdd0/game/CGOV00001");
            }
            other => panic!("the 226 must not answer the RMD: {other:?}"),
        }
    }
    assert_eq!(control.sent(), "DELE /dev_hdd0/tmp/cgov_x.bin\r\n");
}

#[test]
fn a_path_that_could_split_the_command_is_refused_before_anything_is_sent() {
    let mut control = MemoryWire::answering("220 ready\r\n");
    let mut transcript = Transcript::new();
    {
        let mut session = FtpSession::open(&mut control, &mut transcript).expect("greeting");
        for path in ["", "/a b", "/a\r\nDELE /b"] {
            let err = session.rmd(path, &mut transcript).expect_err("refused");
            assert!(
                matches!(err, TransportError::BadArgument { .. }),
                "{path:?}"
            );
        }
    }
    assert!(control.output.is_empty());
}
