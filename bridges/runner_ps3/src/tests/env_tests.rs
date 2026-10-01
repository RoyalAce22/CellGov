//! The lease holder's name: the first variable that names each part.

use super::*;

fn from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_string())
    }
}

#[test]
fn the_windows_names_come_first_and_a_blank_one_does_not_count() {
    assert_eq!(
        who(from(&[
            ("USERNAME", "ana"),
            ("USER", "bo"),
            ("COMPUTERNAME", "BENCH-A"),
            ("HOSTNAME", "bench-b"),
        ])),
        "ana@BENCH-A"
    );
    assert_eq!(
        who(from(&[
            ("USERNAME", " "),
            ("USER", "bo"),
            ("HOSTNAME", "bench-b")
        ])),
        "bo@bench-b"
    );
    assert_eq!(who(from(&[])), "unknown@unknown");
}
