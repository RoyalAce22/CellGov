use super::*;

/// Whether `dir` lives on a filesystem that folds case.
fn folds_case(dir: &Path) -> bool {
    std::fs::write(dir.join("probe.md"), "probe").unwrap();
    let folded = dir.join("PROBE.MD").is_file();
    std::fs::remove_file(dir.join("probe.md")).unwrap();
    folded
}

#[test]
fn a_page_this_run_wrote_is_not_an_orphan_under_the_name_the_directory_kept() {
    let out = cellgov_testkit::scratch::scratch_labeled("orphan-case");
    let dir = out.join(DETAIL_DIR);
    std::fs::create_dir_all(&dir).unwrap();
    let folds = folds_case(&dir);

    // An earlier run left the page under a different spelling of
    // the same content id.
    std::fs::write(dir.join("npaa90007.md"), "old").unwrap();
    let docs = vec![GeneratedDoc {
        path: detail_page_path("NPAA90007"),
        body: "new".to_string(),
    }];
    std::fs::write(out.join(&docs[0].path), &docs[0].body).unwrap();

    let orphans = orphaned_pages(&out, &docs).unwrap();
    if folds {
        assert!(
            orphans.is_empty(),
            "the sweep claimed the page this run wrote: {orphans:?}"
        );
    } else {
        assert_eq!(orphans, vec![dir.join("npaa90007.md")]);
    }
}
