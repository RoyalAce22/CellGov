use super::*;

use cellgov_terminal::caps::RenderMode;

fn caps(mode: RenderMode) -> TermCaps {
    TermCaps {
        mode,
        color: mode == RenderMode::Ansi,
        width: 100,
    }
}

#[test]
fn the_section_trace_drops_an_in_place_frame_to_threshold_lines() {
    let capped = cap_for_trace(caps(RenderMode::Ansi), true);
    assert_eq!(capped.mode, RenderMode::Plain);
    assert!(!capped.color);
    assert_eq!(capped.width, 100);
}

#[test]
fn a_quiet_run_keeps_the_mode_it_detected() {
    for mode in [RenderMode::Off, RenderMode::Plain, RenderMode::Ansi] {
        let kept = cap_for_trace(caps(mode), false);
        assert_eq!(kept.mode, mode);
        assert_eq!(kept.color, caps(mode).color);
    }
}

#[test]
fn the_modes_that_never_cursor_up_are_left_alone_by_the_trace() {
    for mode in [RenderMode::Off, RenderMode::Plain] {
        assert_eq!(cap_for_trace(caps(mode), true).mode, mode);
    }
}

/// Every `.rs` file under `dir`, relative to it, in `/`-separated form.
fn rust_sources(dir: &std::path::Path, prefix: &str, out: &mut Vec<(String, std::path::PathBuf)>) {
    for entry in std::fs::read_dir(dir).expect("store source directory") {
        let path = entry.expect("store source entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("store source name")
            .to_string();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if path.is_dir() {
            rust_sources(&path, &rel, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push((rel, path));
        }
    }
}

/// A store command that resolves the flags itself skips the trace
/// question.
#[test]
fn only_this_module_resolves_render_flags_for_a_store_command() {
    let needle = concat!("render", ".caps()");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli/store");
    let mut sources = Vec::new();
    rust_sources(&dir, "", &mut sources);
    assert!(
        sources.len() > 20,
        "the scan found {} files in {dir:?}; it is not reading the store tree",
        sources.len()
    );
    let resolving: std::collections::BTreeSet<String> = sources
        .into_iter()
        .filter(|(_, path)| {
            std::fs::read_to_string(path)
                .expect("store source")
                .contains(needle)
        })
        .map(|(rel, _)| rel)
        .collect();
    assert_eq!(
        resolving,
        std::collections::BTreeSet::from(["container.rs".to_string()]),
        "a store command must start its bar from install_caps, not resolve the flags itself"
    );
}
