use super::*;

#[test]
fn renderer_replaces_only_marked_regions() {
    let metadata = Metadata {
        packages: vec![
            Package {
                name: "a".to_string(),
                dependencies: vec![],
            },
            Package {
                name: "b".to_string(),
                dependencies: vec![
                    Dependency {
                        name: "a".to_string(),
                        source: None,
                        path: Some(PathBuf::from("a")),
                        kind: None,
                        target: None,
                    },
                    Dependency {
                        name: "serde".to_string(),
                        source: Some("registry".to_string()),
                        path: None,
                        kind: None,
                        target: None,
                    },
                ],
            },
        ],
    };
    let input = format!(
        "before\n{DAG_START}\nold\n{DAG_END}\nmid\n{EXTERNAL_START}\nold\n{EXTERNAL_END}\nafter\n"
    );
    let rendered = render_document(&input, &metadata).expect("marked regions render");
    assert!(rendered.contains("n0[\"a\"]"));
    assert!(rendered.contains("n0 --> n1"));
    assert!(rendered.contains("| `b` | serde |"));
    assert!(rendered.contains("before\n"));
    assert!(rendered.contains("\nmid\n"));
    assert!(rendered.ends_with("after\n"));
}

#[test]
fn renderer_refuses_duplicate_markers() {
    let metadata = Metadata { packages: vec![] };
    let input =
        format!("{DAG_START}\n{DAG_END}\n{DAG_START}\n{DAG_END}\n{EXTERNAL_START}\n{EXTERNAL_END}");
    let error = render_document(&input, &metadata).expect_err("duplicate DAG marker refuses");
    assert_eq!(error, format!("expected one {DAG_START}"));
}
