//! What the step loop declares to its sink on entry.

use std::sync::Mutex;

use super::*;

/// A sink that records every call, in order.
struct Recording(Mutex<Vec<String>>);

impl Recording {
    fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    fn push(&self, event: String) {
        self.0.lock().expect("recording sink").push(event);
    }

    fn events(&self) -> Vec<String> {
        self.0.lock().expect("recording sink").clone()
    }
}

impl ProgressSink for Recording {
    fn phase(&self, code: u8) {
        self.push(format!("phase {code}"));
    }
    fn totals(&self, items: usize, amount: u64) {
        self.push(format!("totals {items} {amount}"));
    }
    fn preset_done(&self, amount: u64) {
        self.push(format!("preset_done {amount}"));
    }
    fn item_started(&self, name: &str) {
        self.push(format!("item_started {name}"));
    }
    fn advanced(&self, delta: u64) {
        self.push(format!("advanced {delta}"));
    }
    fn item_finished(&self) {
        self.push("item_finished".to_string());
    }
    fn finished(&self) {
        self.push("finished".to_string());
    }
}

#[test]
fn a_boot_with_a_finish_line_declares_it_before_the_step_phase() {
    let sink = Recording::new();
    enter_step_loop(&sink, Some(43_040));
    assert_eq!(
        sink.events(),
        vec![
            "totals 0 43040".to_string(),
            format!("phase {}", BootPhase::Stepping.code()),
        ]
    );
}

#[test]
fn a_boot_without_a_finish_line_declares_only_the_step_phase() {
    let sink = Recording::new();
    enter_step_loop(&sink, None);
    assert_eq!(
        sink.events(),
        vec![format!("phase {}", BootPhase::Stepping.code())]
    );
}
