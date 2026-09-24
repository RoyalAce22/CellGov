use super::build_sql;

#[test]
fn census_import_order_does_not_change_the_build_script() {
    let ascending = vec![
        "census/fw-3.55.tsv".to_string(),
        "census/fw-3.56.tsv".to_string(),
    ];
    let descending = ascending.iter().rev().cloned().collect::<Vec<_>>();
    assert_eq!(build_sql(&ascending), build_sql(&descending));
}
