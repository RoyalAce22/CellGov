//! The title walks of the installed-title suites, with the boot or the
//! anchor read passed in, so their skipping and ordering hold without a
//! title installed.
//!
//! The including crate declares `registry` beside this module.
#![allow(
    dead_code,
    reason = "each integration-test binary compiles this module separately and uses a subset"
)]
#![allow(
    clippy::print_stderr,
    reason = "each walk names the cells it sets aside on stderr"
)]

use cellgov_ps3_abi::format::sce::BDJ_SELF_PROGRAM_AUTHORITY_ID;

use super::registry::TitleUnderTest;

/// What a boot reports about the authority id it served.
pub struct AuthorityWitness {
    pub program_authority_id: u64,
    pub authid_source: String,
    pub lwmutex_unknown_locks: u64,
}

/// Boot each runnable title twice, under its own authority id and under
/// the forced system-class one, and hold the pair to the class pin.
///
/// `boot_title` returns `None` for a title that is not installed.
///
/// # Panics
///
/// When a booted title breaks the pin, when no title booted, or when
/// no installed title shows the forced id's regression signature.
pub fn check_titles(
    titles: &[TitleUnderTest],
    mut boot_title: impl FnMut(&TitleUnderTest, bool) -> Option<AuthorityWitness>,
) {
    let mut checked = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    let mut any_forced_delta = false;
    for title in titles {
        if let Some(why) = &title.reference.pending {
            eprintln!(
                "{} ({}): skipped -- declared pending ({why})",
                title.short_name,
                title.reference.label()
            );
            skipped.push(format!("{} {}", title.short_name, title.reference.label()));
            continue;
        }
        let Some(normal) = boot_title(title, false) else {
            eprintln!(
                "{} ({}): skipped -- not installed on this machine",
                title.short_name,
                title.reference.label()
            );
            skipped.push(format!("{} {}", title.short_name, title.reference.label()));
            continue;
        };
        checked += 1;
        assert_eq!(
            normal.authid_source, "self",
            "{}: the served authority id must come from the SELF \
             identification header, not the shared retail fallback -- \
             a fallback here means the SELF parse regressed",
            title.short_name,
        );
        assert_ne!(
            normal.program_authority_id, BDJ_SELF_PROGRAM_AUTHORITY_ID,
            "{}: normal boot served the system-class bdj.self id; \
             titles must boot under a retail-class authority id",
            title.short_name,
        );
        assert_eq!(
            normal.lwmutex_unknown_locks, 0,
            "{}: unknown-lwmutex lock failures under a retail-class authid",
            title.short_name,
        );
        let forced = boot_title(title, true).expect("installed above; forced boot must also start");
        assert_eq!(
            forced.program_authority_id, BDJ_SELF_PROGRAM_AUTHORITY_ID,
            "{}: --force-system-authid must serve the bdj.self constant",
            title.short_name,
        );
        if forced.lwmutex_unknown_locks > normal.lwmutex_unknown_locks {
            any_forced_delta = true;
        }
    }
    // Anti-vacuity floor, shared with title_witnesses: the feature
    // declares installed titles, so a run that booted nothing must not pass.
    assert!(
        checked > 0,
        "installed-title-tests is enabled but none of the {} registered title(s) is \
         installed (skipped: {})",
        titles.len(),
        skipped.join(", ")
    );
    // Negative control: the regression signature the `== 0` asserts
    // gate must still be reproducible somewhere in the installed set.
    assert!(
        any_forced_delta,
        "no installed title shows more unknown-lwmutex failures under the \
         forced system authid than under its own id; the class pin has lost \
         its negative control and the `== 0` assertions are unfalsifiable"
    );
}

/// The runnable titles, cheapest `anchor_steps` first. A pending cell
/// is set aside before its anchor is read.
pub fn by_cost_from(
    titles: Vec<TitleUnderTest>,
    mut anchor_steps: impl FnMut(&TitleUnderTest) -> u64,
) -> Vec<(u64, TitleUnderTest)> {
    let mut out = Vec::new();
    for title in titles {
        if let Some(why) = &title.reference.pending {
            eprintln!(
                "SKIP pending: {} ({}): {why}",
                title.short_name,
                title.reference.label()
            );
            continue;
        }
        out.push((anchor_steps(&title), title));
    }
    out.sort_by_key(|(steps, _)| *steps);
    out
}
