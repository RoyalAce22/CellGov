//! `CURATED` reconciled by name against the `pub mod` blocks in `modules.rs`.

use super::*;

/// Module names declared in `modules.rs`, in source order.
fn declared_module_names() -> Vec<&'static str> {
    include_str!("../modules.rs")
        .lines()
        .filter_map(|line| line.strip_prefix("pub mod "))
        .map(|rest| {
            rest.split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("")
        })
        .collect()
}

#[test]
fn every_module_in_modules_rs_has_exactly_one_curated_row() {
    let names = declared_module_names();
    assert!(!names.is_empty(), "modules.rs declares no `pub mod`");
    let mut wrong = Vec::new();
    for name in &names {
        let rows = CURATED.iter().filter(|(m, _)| m == name).count();
        if rows != 1 {
            wrong.push(format!("{name}: {rows} CURATED rows, expected 1"));
        }
    }
    for (m, _) in CURATED {
        if !names.contains(m) {
            wrong.push(format!(
                "{m}: CURATED row with no `pub mod {m}` in modules.rs"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
