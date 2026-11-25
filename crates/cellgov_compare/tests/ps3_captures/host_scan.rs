//! The transcript scan for the console's address. The runner's redactor
//! writes the address as `<host>`, so four decimal fields in a committed
//! transcript are a leak: joined by dots as an IPv4 address, or by
//! commas as the host of a `227` reply to `PASV`.

/// The fields of an IPv4 address.
const HOST_FIELDS: usize = 4;

/// Each transcript line that holds four decimal fields joined by dots,
/// or by commas with optional spaces.
pub fn host_leaks(transcript: &str) -> Vec<String> {
    let mut leaks = Vec::new();
    for (n, line) in transcript.lines().enumerate() {
        if holds_host_fields(line, '.', false) {
            leaks.push(format!("line {}: a dotted IPv4 address", n + 1));
        }
        if holds_host_fields(line, ',', true) {
            leaks.push(format!("line {}: a comma-form host address", n + 1));
        }
    }
    leaks
}

/// Whether `line` holds [`HOST_FIELDS`] runs of one to three digits in a
/// row, each joined to the next by `separator`. With `spaces`, a space
/// may stand on either side of a separator.
fn holds_host_fields(line: &str, separator: char, spaces: bool) -> bool {
    let words =
        line.split(|c: char| !(c.is_ascii_digit() || c == separator || (spaces && c == ' ')));
    words.into_iter().any(|word| {
        let mut run = 0;
        for field in word.split(separator) {
            let field = field.trim_matches(' ');
            let decimal =
                (1..=3).contains(&field.len()) && field.bytes().all(|b| b.is_ascii_digit());
            run = if decimal { run + 1 } else { 0 };
            if run == HOST_FIELDS {
                return true;
            }
        }
        false
    })
}

#[test]
fn the_host_scan_finds_each_address_form_and_spares_the_placeholder() {
    let transcript = concat!(
        "#0001 < 227 Entering Passive Mode (192,0,2,7,130,248)\n",
        "#0002 < 227 Entering Passive Mode (198, 51, 100, 4, 4, 1)\n",
        "#0003 > PORT 203,0,113,9,4,1\n",
        "#0004 = status page unanswered: connect to 192.0.2.7:80: refused\n",
        "#0005 < 227 Entering Passive Mode (<host>,130,248)\n",
        "#0006 = console: webman 1.47.48t, firmware 4.93, cobra 8.5\n",
        "#0007 = load: Cell 67 C, RSX 63 C, fan 37%; cool\n",
        "#0008 = connect to <host>:80: refused\n",
    );
    assert_eq!(
        host_leaks(transcript),
        [
            "line 1: a comma-form host address",
            "line 2: a comma-form host address",
            "line 3: a comma-form host address",
            "line 4: a dotted IPv4 address",
        ]
    );
}
