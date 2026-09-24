use std::io::Write;

use super::*;

/// A writer that accepts `budget` bytes and refuses every write after.
struct Refusing {
    written: Vec<u8>,
    budget: usize,
}

impl Write for Refusing {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.written.len() + buf.len() > self.budget {
            return Err(std::io::Error::other("disk full"));
        }
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn the_header_leads_and_each_record_follows_it_whole() {
    let mut file = RecordFile::over(Vec::new(), b"HEAD").unwrap();
    file.append(b"one").unwrap();
    file.append(b"two").unwrap();
    assert_eq!(file.into_inner(), b"HEADonetwo");
}

#[test]
fn the_first_failed_write_is_returned_once_and_ends_the_capture() {
    let out = Refusing {
        written: Vec::new(),
        budget: 7,
    };
    let mut file = RecordFile::over(out, b"HEAD").unwrap();
    file.append(b"abc").unwrap();
    let err = file.append(b"def").expect_err("the budget is spent");
    assert_eq!(err.to_string(), "disk full");
    file.append(b"")
        .expect("after the failure, nothing is written");
    file.append(b"g")
        .expect("after the failure, nothing is written");
    assert_eq!(file.into_inner().written, b"HEADabc");
}

#[test]
fn a_failure_is_held_until_taken_and_the_first_one_wins() {
    let mut first = FirstFailure::default();
    first.note(Ok(()));
    assert!(first.take().is_none());
    first.note(Err(std::io::Error::other("first")));
    first.note(Err(std::io::Error::other("second")));
    assert_eq!(
        first.take().map(|e| e.to_string()).as_deref(),
        Some("first")
    );
    assert!(first.take().is_none(), "taken once");
}
