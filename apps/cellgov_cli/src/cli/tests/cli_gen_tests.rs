use super::*;

struct ClosedPipe;

impl Write for ClosedPipe {
    fn write(&mut self, _body: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_closed_completion_pipe_returns_status_without_a_diagnostic() {
    let code = write_body(ClosedPipe, b"completion script")
        .expect("a closed downstream reader is a status outcome");
    assert_eq!(code, CommandExitCode::new(exit_codes::BROKEN_PIPE));
}
