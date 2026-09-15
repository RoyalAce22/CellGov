//! What `collect_override_env` reads, and what it refuses by name.

use std::env::VarError;

use super::{collect_override_env, ProviderError};

fn names(raw: &[&str]) -> Vec<String> {
    raw.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn an_absent_variable_is_left_out_of_the_map() {
    let env = collect_override_env(names(&["CELLGOV_A"]), |_| Err(VarError::NotPresent))
        .expect("an absent variable is not a refusal");
    assert!(env.is_empty(), "an absent name must not land as a key");
    assert_eq!(env.get("CELLGOV_A"), None);
}

#[test]
fn a_blank_value_is_carried_through_rather_than_dropped() {
    let env = collect_override_env(names(&["CELLGOV_A"]), |_| Ok(String::new()))
        .expect("a blank value is not a refusal");
    assert_eq!(env.get("CELLGOV_A"), Some(&String::new()));
}

#[test]
fn one_name_declared_twice_is_read_into_one_entry() {
    let env = collect_override_env(names(&["CELLGOV_A", "CELLGOV_A"]), |_| {
        Ok("/roots".to_string())
    })
    .expect("a repeated name is not a refusal");
    assert_eq!(env.len(), 1, "the map collapses a shared variable");
    assert_eq!(env.get("CELLGOV_A"), Some(&"/roots".to_string()));
}

#[test]
fn a_value_that_is_not_unicode_is_refused_naming_the_variable() {
    let err = collect_override_env(names(&["CELLGOV_A"]), |_| {
        Err(VarError::NotUnicode("bad".into()))
    })
    .expect_err("a value that is not Unicode is a refusal");
    match err {
        ProviderError::OverrideNotUnicode { name, .. } => assert_eq!(name, "CELLGOV_A"),
        other => panic!("expected OverrideNotUnicode, got {other:?}"),
    }
}

#[test]
fn a_later_refusal_does_not_discard_the_names_read_before_it() {
    let err = collect_override_env(names(&["CELLGOV_A", "CELLGOV_B"]), |name| {
        if name == "CELLGOV_B" {
            Err(VarError::NotUnicode("bad".into()))
        } else {
            Ok("/roots".to_string())
        }
    })
    .expect_err("a value that is not Unicode is a refusal");
    match err {
        ProviderError::OverrideNotUnicode { name, .. } => assert_eq!(name, "CELLGOV_B"),
        other => panic!("expected OverrideNotUnicode, got {other:?}"),
    }
}

#[test]
fn no_declared_variable_reads_nothing() {
    let env = collect_override_env(Vec::<String>::new(), |_| -> Result<String, VarError> {
        panic!("no name was declared, so nothing may be read")
    })
    .expect("an empty declaration list is not a refusal");
    assert!(env.is_empty());
}
