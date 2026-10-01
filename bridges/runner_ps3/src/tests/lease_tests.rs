//! The lease on the console: one holder at a time across every runner
//! that reaches it, the refusal that names the holder and the clearing
//! command, release and unlock.

use super::*;
use crate::memory_console::MemoryConsole;
use crate::run::RESULT_ROOT;

const HOST: &str = "10.77.0.2";

fn take(console: &mut MemoryConsole, who: &str, test: &str) -> Result<Lease, LeaseError> {
    Lease::acquire(
        console,
        HOST,
        &holder_text(who, test, "cellgov ps3"),
        "cellgov ps3",
        &mut Transcript::new(),
    )
}

#[test]
fn the_lease_paths_sit_under_the_result_root() {
    assert_eq!(LEASE_DIR, format!("{RESULT_ROOT}/{LEASE_NAME}"));
    assert_eq!(HOLDER_FILE, format!("{LEASE_DIR}/holder"));
}

#[test]
fn a_second_runner_on_any_machine_is_refused_with_the_holder_and_the_command_that_clears_it() {
    let mut console = MemoryConsole::empty();
    let first = take(&mut console, "ana@bench-a", "spu_fixed_value").expect("free");
    assert_eq!(
        console.files[HOLDER_FILE],
        format!(
            "ana@bench-a, pid {}, test spu_fixed_value, via cellgov ps3\n",
            std::process::id()
        )
        .into_bytes()
    );
    let err = take(&mut console, "bo@bench-b", "dma_completion").expect_err("held");
    match &err {
        LeaseError::Held { host, holder, .. } => {
            assert_eq!(host, HOST);
            assert!(
                holder.starts_with("ana@bench-a, pid ")
                    && holder.ends_with(", test spu_fixed_value, via cellgov ps3"),
                "{holder}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        err.to_string()
            .ends_with("clear it with `cellgov ps3 unlock --host 10.77.0.2`"),
        "{err}"
    );
    first
        .release(&mut console, &mut Transcript::new())
        .expect("release");
    assert!(!console.dirs.contains(LEASE_DIR));
    assert!(!console.files.contains_key(HOLDER_FILE));
    drop(take(&mut console, "bo@bench-b", "dma_completion").expect("free again"));
}

#[test]
fn a_lease_directory_without_a_holder_file_still_refuses() {
    let mut console = MemoryConsole::empty();
    console.dirs.insert(LEASE_DIR.to_string());
    match take(&mut console, "ana@bench-a", "a").expect_err("held") {
        LeaseError::Held { holder, .. } => {
            assert!(holder.starts_with("an unnamed holder"), "{holder}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_holder_that_does_not_store_gives_the_lease_back() {
    let console = MemoryConsole::empty();
    // A store into a directory the console lost fails; model it by
    // taking the directory away between the MKD and the STOR.
    struct LosesTheDir(MemoryConsole);
    impl ConsoleOps for LosesTheDir {
        fn http_status(&mut self, p: &str, t: &mut Transcript) -> Result<u16, TransportError> {
            self.0.http_status(p, t)
        }
        fn fetch(
            &mut self,
            p: &str,
            t: &mut Transcript,
        ) -> Result<Option<Vec<u8>>, TransportError> {
            self.0.fetch(p, t)
        }
        fn make_dir(&mut self, p: &str, t: &mut Transcript) -> Result<(), TransportError> {
            self.0.make_dir(p, t)?;
            self.0.dirs.remove(p);
            Ok(())
        }
        fn store(&mut self, p: &str, b: &[u8], t: &mut Transcript) -> Result<(), TransportError> {
            self.0.store(p, b, t)
        }
        fn list(&mut self, d: &str, t: &mut Transcript) -> Result<Vec<String>, TransportError> {
            self.0.list(d, t)
        }
        fn delete(&mut self, p: &str, t: &mut Transcript) -> Result<(), TransportError> {
            self.0.delete(p, t)
        }
        fn remove_dir(&mut self, p: &str, t: &mut Transcript) -> Result<(), TransportError> {
            self.0.remove_dir(p, t)
        }
    }
    let mut losing = LosesTheDir(console);
    let err = Lease::acquire(
        &mut losing,
        HOST,
        "ana@bench-a",
        "runner_ps3",
        &mut Transcript::new(),
    )
    .expect_err("the store fails");
    assert!(matches!(err, LeaseError::Transport(_)), "{err:?}");
    assert_eq!(
        losing.0.calls,
        [
            format!("MKD {LEASE_DIR}"),
            format!("STOR {HOLDER_FILE} (11 bytes)"),
            format!("RMD {LEASE_DIR}"),
        ],
        "the runner tries to give the directory back"
    );
}

#[test]
fn unlock_clears_a_stale_lease_and_reports_whether_one_was_there() {
    let mut console = MemoryConsole::empty();
    // A run that stops without releasing: the lease stays on the console.
    drop(take(&mut console, "ana@bench-a", "a").expect("free"));
    assert!(unlock(&mut console, &mut Transcript::new()).expect("unlock"));
    assert!(!console.dirs.contains(LEASE_DIR));
    assert!(!unlock(&mut console, &mut Transcript::new()).expect("nothing to unlock"));
    drop(take(&mut console, "bo@bench-b", "b").expect("free after unlock"));
}

#[test]
fn unlock_clears_a_lease_with_no_holder_file() {
    let mut console = MemoryConsole::empty();
    console.dirs.insert(LEASE_DIR.to_string());
    assert!(unlock(&mut console, &mut Transcript::new()).expect("unlock"));
    drop(take(&mut console, "ana@bench-a", "a").expect("free after unlock"));
}
