//! The CLI's rendering of a boot's narration.

use std::rc::Rc;

use cellgov_boot::BootSink;

/// Writes a boot's narration to the console.
///
/// The guest channel flushes, so a fault stack on stderr cannot
/// interleave with the output that preceded it.
struct ConsoleSink;

impl BootSink for ConsoleSink {
    fn note(&self, line: &str) {
        println!("{line}");
    }

    fn warn(&self, line: &str) {
        eprintln!("{line}");
    }

    fn guest_text(&self, text: &str) {
        use std::io::Write;
        print!("{text}");
        let _ = std::io::stdout().flush();
    }
}

/// The sink both boot commands write their narration to.
pub(crate) fn console_sink() -> Rc<dyn BootSink> {
    Rc::new(ConsoleSink)
}
