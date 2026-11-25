//! The redactor keeps the console's address out of the transcript in
//! each form a run can carry. Every address is a documentation address
//! (RFC 5737).

use super::*;

#[test]
fn a_pasv_reply_keeps_its_port_fields_and_masks_its_host() {
    assert_eq!(
        redact("227 Entering Passive Mode (192,0,2,7,130,248)"),
        "227 Entering Passive Mode (<host>,130,248)"
    );
    assert_eq!(
        redact("227 Entering Passive Mode (192, 0, 2, 7, 130, 248)."),
        "227 Entering Passive Mode (<host>, 130, 248)."
    );
    assert_eq!(
        redact("227 Entering Passive Mode 198,51,100,4,4,1"),
        "227 Entering Passive Mode <host>,4,1"
    );
}

#[test]
fn a_port_command_keeps_its_port_fields_and_masks_its_host() {
    assert_eq!(redact("PORT 192,0,2,7,4,1"), "PORT <host>,4,1");
}

#[test]
fn a_dotted_address_is_masked_and_its_port_kept() {
    assert_eq!(
        redact("status page unanswered: connect to 192.0.2.7:80: refused"),
        "status page unanswered: connect to <host>:80: refused"
    );
    assert_eq!(redact("EPRT |1|198.51.100.4|6275|"), "EPRT |1|<host>|6275|");
    assert_eq!(
        redact("the console is 192.0.2.7."),
        "the console is <host>."
    );
}

#[test]
fn numbers_that_are_not_an_address_are_kept() {
    for text in [
        "229 Entering Extended Passive Mode (|||6446|)",
        "webman 1.47.48t, firmware 4.93, cobra 8.5",
        "version 1.2.3.4.5",
        "v192.0.2.7",
        "256.0.2.7",
        "1,2,3,4,5,6,7",
        "1,2,3,4,5",
        "(192,0,2,7,256,1)",
    ] {
        assert_eq!(redact(text), text);
    }
}

#[test]
fn the_console_host_name_is_masked_where_it_stands_alone() {
    let mut transcript = Transcript::for_host(" ps3 ");
    transcript.request("GET /cpursx.ps3");
    transcript.decision("status page unanswered: resolve PS3:80: no such host");
    transcript.decision("ps3-lab and lab.ps3.example are other hosts");
    transcript.decision("ps3.lab is another host; the console is ps3.");
    assert_eq!(
        transcript.lines(),
        [
            "#0001 > GET /cpursx.ps3",
            "#0002 = status page unanswered: resolve <host>:80: no such host",
            "#0003 = ps3-lab and lab.ps3.example are other hosts",
            "#0004 = ps3.lab is another host; the console is <host>.",
        ]
    );
}

#[test]
fn a_transcript_for_a_dotted_host_leaves_neither_form_of_it() {
    let mut transcript = Transcript::for_host("192.0.2.7");
    transcript.request("PASV");
    transcript.reply("227 Entering Passive Mode (192,0,2,7,195,80)");
    transcript.decision("status page unanswered: connect to 192.0.2.7:80: refused");
    assert_eq!(
        transcript.lines(),
        [
            "#0001 > PASV",
            "#0002 < 227 Entering Passive Mode (<host>,195,80)",
            "#0003 = status page unanswered: connect to <host>:80: refused",
        ]
    );
}

#[test]
fn a_blank_host_masks_nothing_more_than_no_host() {
    let mut blank = Transcript::for_host("  ");
    let mut none = Transcript::new();
    for transcript in [&mut blank, &mut none] {
        transcript.request("GET /cpursx.ps3");
    }
    assert_eq!(blank, none);
}
