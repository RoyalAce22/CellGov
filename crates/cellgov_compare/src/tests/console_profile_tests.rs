//! Console profiles: the tracked file, the hard-field check, soft fields
//! that never refuse, the refusal that names another satisfied profile,
//! and the load-time refusals.

use std::path::Path;

use super::*;

const PROFILES: &str = r#"
reference = "cech20-cex-493"

[profile.cech20-cex-493]
models = ["CECH-20"]
kernel = "cex"
firmware = "4.93"
cfw = "EvilNAT"
cobra = "8.5"
debugger_attached = false
"#;

fn console() -> ConsoleFacts {
    ConsoleFacts {
        profile: "cech20-cex-493".to_string(),
        model: "CECH-2001A".to_string(),
        kernel: "cex".to_string(),
        firmware: "4.93".to_string(),
        cfw: "EvilNAT 4.93 PEX".to_string(),
        cobra: "8.5".to_string(),
        webman: Some("1.47.48t".to_string()),
        debugger_attached: false,
    }
}

fn profiles(text: &str) -> ConsoleProfiles {
    ConsoleProfiles::parse(text).expect("parses")
}

#[test]
fn the_tracked_file_loads_and_the_reference_console_satisfies_it() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(CONSOLE_PROFILES_FILE);
    let tracked = ConsoleProfiles::load(&path).expect("the tracked profiles load");
    assert_eq!(
        tracked,
        profiles(PROFILES),
        "the test copy matches the file"
    );
    tracked
        .check(&tracked.reference, &console())
        .expect("the reference console satisfies the reference profile");
}

#[test]
fn a_wrong_hard_field_refuses_naming_the_profile_the_field_and_both_values() {
    let wrong = profiles(&PROFILES.replace("firmware = \"4.93\"", "firmware = \"4.92\""));
    let err = wrong.check("cech20-cex-493", &console()).expect_err("4.92");
    match &err {
        ConsoleProfileError::Mismatch {
            profile,
            mismatches,
            satisfied,
        } => {
            assert_eq!(profile, "cech20-cex-493");
            assert_eq!(
                mismatches,
                &[FieldMismatch {
                    field: "firmware",
                    expected: "\"4.92\"".to_string(),
                    observed: "4.93".to_string(),
                }]
            );
            assert!(satisfied.is_empty());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        err.to_string(),
        "the console does not satisfy profile \"cech20-cex-493\": firmware is \"4.93\", \
         the profile requires \"4.92\"; no tracked profile matches it; add one to \
         tests/micro/console_profiles.toml"
    );
}

#[test]
fn soft_fields_never_refuse() {
    let mut other = console();
    other.webman = Some("1.47.50".to_string());
    other.model = "CECH-2004B".to_string();
    other.cfw = "EvilNAT 4.93.2 PEX".to_string();
    profiles(PROFILES)
        .check("cech20-cex-493", &other)
        .expect("only soft fields differ");
}

#[test]
fn a_refusal_names_every_other_profile_the_console_satisfies() {
    let text = format!(
        "{}\n[profile.cech20-cex-493-cobra84]\nmodels = [\"CECH-20\"]\nkernel = \"cex\"\n\
         firmware = \"4.93\"\ncfw = \"EvilNAT\"\ncobra = \"8.4\"\ndebugger_attached = false\n",
        PROFILES
    );
    let two = profiles(&text);
    let mut console = console();
    console.cobra = "8.4".to_string();
    let err = two
        .check("cech20-cex-493", &console)
        .expect_err("cobra 8.4");
    match &err {
        ConsoleProfileError::Mismatch {
            mismatches,
            satisfied,
            ..
        } => {
            assert_eq!(mismatches.len(), 1);
            assert_eq!(mismatches[0].field, "cobra");
            assert_eq!(satisfied, &["cech20-cex-493-cobra84"]);
        }
        other => panic!("{other:?}"),
    }
    assert!(
        err.to_string()
            .ends_with("; it satisfies --profile cech20-cex-493-cobra84"),
        "{err}"
    );
}

#[test]
fn every_hard_field_is_checked_in_file_order() {
    let foreign = ConsoleFacts {
        profile: "cech20-cex-493".to_string(),
        model: "CECH-2501A".to_string(),
        kernel: "dex".to_string(),
        firmware: "4.92".to_string(),
        cfw: "EvilNATX 4.93".to_string(),
        cobra: "8.4".to_string(),
        webman: None,
        debugger_attached: true,
    };
    let fields: Vec<&str> = profiles(PROFILES).profile["cech20-cex-493"]
        .mismatches(&foreign)
        .iter()
        .map(|m| m.field)
        .collect();
    assert_eq!(
        fields,
        [
            "models",
            "kernel",
            "firmware",
            "cfw",
            "cobra",
            "debugger_attached"
        ]
    );
}

#[test]
fn the_cfw_name_matches_alone_or_before_a_build_string() {
    assert!(cfw_matches("EvilNAT", "EvilNAT"));
    assert!(cfw_matches("EvilNAT 4.93 PEX", "EvilNAT"));
    assert!(!cfw_matches("EvilNATX 4.93", "EvilNAT"));
    assert!(!cfw_matches("Rebug 4.93", "EvilNAT"));
}

#[test]
fn an_unknown_field_is_refused_at_load() {
    for text in [
        PROFILES.replace("cobra = \"8.5\"", "cobra = \"8.5\"\nwebman = \"1.47.48t\""),
        format!("owner = \"x\"\n{PROFILES}"),
    ] {
        assert!(
            matches!(
                ConsoleProfiles::parse(&text),
                Err(ConsoleProfileError::Parse(_))
            ),
            "{text}"
        );
    }
}

#[test]
fn a_missing_reference_and_an_empty_or_blank_model_list_are_refused_at_load() {
    assert!(matches!(
        ConsoleProfiles::parse(&PROFILES.replace(
            "reference = \"cech20-cex-493\"",
            "reference = \"cech25\""
        )),
        Err(ConsoleProfileError::UnknownReference(name)) if name == "cech25"
    ));
    assert!(matches!(
        ConsoleProfiles::parse(&PROFILES.replace("[\"CECH-20\"]", "[]")),
        Err(ConsoleProfileError::NoModels(name)) if name == "cech20-cex-493"
    ));
    assert!(matches!(
        ConsoleProfiles::parse(&PROFILES.replace("[\"CECH-20\"]", "[\"CECH-20\", \"\"]")),
        Err(ConsoleProfileError::NoModels(name)) if name == "cech20-cex-493"
    ));
}

#[test]
fn an_untracked_claim_names_the_tracked_profiles() {
    match profiles(PROFILES).check("cech25-dex-493", &console()) {
        Err(ConsoleProfileError::UnknownProfile { claimed, known }) => {
            assert_eq!(claimed, "cech25-dex-493");
            assert_eq!(known, "cech20-cex-493");
        }
        other => panic!("{other:?}"),
    }
}
