use super::*;

fn path_dep(name: &str) -> Dependency {
    Dependency {
        name: name.to_string(),
        source: None,
        path: Some(PathBuf::from(name)),
        kind: None,
        target: None,
    }
}

fn package(name: &str, dependencies: Vec<Dependency>) -> Package {
    Package {
        name: name.to_string(),
        dependencies,
    }
}

/// `c` depends on `b` and, redundantly, on `a`, which `b` already
/// depends on.
fn three_crates() -> Metadata {
    Metadata {
        packages: vec![
            package("cellgov_a", vec![]),
            package(
                "cellgov_b",
                vec![
                    path_dep("cellgov_a"),
                    Dependency {
                        name: "serde".to_string(),
                        source: Some("registry".to_string()),
                        path: None,
                        kind: None,
                        target: None,
                    },
                ],
            ),
            package(
                "cellgov_c",
                vec![path_dep("cellgov_a"), path_dep("cellgov_b")],
            ),
        ],
    }
}

const THREE_LAYERS: Layers<'static> = &[("Bottom", &["a"]), ("Top", &["b", "c"])];

#[test]
fn renderer_replaces_only_marked_regions() {
    let input = format!(
        "before\n{DAG_START}\nold\n{DAG_END}\nmid\n{EXTERNAL_START}\nold\n{EXTERNAL_END}\nafter\n"
    );
    let rendered =
        render_document(&input, &three_crates(), THREE_LAYERS).expect("marked regions render");
    assert!(rendered.contains("  subgraph Bottom\n    a\n  end\n"));
    assert!(rendered.contains("  subgraph Top\n    b; c\n  end\n"));
    assert!(rendered.contains("| `cellgov_b` | serde                        |"));
    assert!(rendered.contains("before\n"));
    assert!(rendered.contains("\nmid\n"));
    assert!(rendered.ends_with("after\n"));
}

/// The diagram draws only the edges no longer path explains.
#[test]
fn an_implied_dependency_is_left_out_of_the_diagram() {
    let dag = render_dag(&three_crates(), THREE_LAYERS).expect("renders");
    let edges: Vec<&str> = dag.lines().filter(|l| l.contains("-->")).collect();
    assert_eq!(edges, ["  a --> b", "  b --> c"]);
}

#[test]
fn a_member_outside_every_layer_is_refused() {
    let error = render_dag(&three_crates(), &[("Bottom", &["a", "b"])])
        .expect_err("an unplaced member refuses");
    assert_eq!(error, WorkspaceGenError::Unplaced("c".to_string()));
}

#[test]
fn a_layer_naming_no_member_is_refused() {
    let error = render_dag(&three_crates(), &[("Only", &["a", "b", "c", "d"])])
        .expect_err("a stale layer entry refuses");
    assert_eq!(error, WorkspaceGenError::NotAMember("d".to_string()));
}

#[test]
fn a_crate_placed_in_two_layers_is_refused() {
    let error = render_dag(
        &three_crates(),
        &[("Bottom", &["a", "b"]), ("Top", &["b", "c"])],
    )
    .expect_err("a doubly placed crate refuses");
    assert_eq!(error, WorkspaceGenError::PlacedTwice("b".to_string()));
}

#[test]
fn a_dependency_on_a_higher_layer_is_refused() {
    let error = render_dag(&three_crates(), &[("Bottom", &["b", "c"]), ("Top", &["a"])])
        .expect_err("a downward edge refuses");
    assert_eq!(
        error,
        WorkspaceGenError::UpwardEdge {
            from: "b".to_string(),
            to: "a".to_string()
        }
    );
}

#[test]
fn the_table_pads_every_column_to_its_widest_cell() {
    let table = render_external_dependencies(&three_crates());
    let widths: BTreeSet<usize> = table
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::len)
        .collect();
    assert_eq!(widths.len(), 1, "every row is one width: {table}");
}

#[test]
fn renderer_refuses_duplicate_markers() {
    let metadata = Metadata { packages: vec![] };
    let input =
        format!("{DAG_START}\n{DAG_END}\n{DAG_START}\n{DAG_END}\n{EXTERNAL_START}\n{EXTERNAL_END}");
    let error = render_document(&input, &metadata, &[]).expect_err("duplicate DAG marker refuses");
    assert_eq!(error, WorkspaceGenError::Marker(DAG_START));
    assert_eq!(error.to_string(), format!("expected one {DAG_START}"));
}
