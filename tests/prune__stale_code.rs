use comemory::prune::stale_code;

#[test]
fn stale_code_returns_empty_on_empty_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let missing = stale_code::detect(dir.path()).unwrap();
    assert!(
        missing.is_empty(),
        "v1 stale_code::detect always returns empty (no memory walk yet), got {missing:?}",
    );
}

#[test]
fn stale_code_returns_empty_on_nonexistent_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let bogus = dir.path().join("does-not-exist");
    let missing = stale_code::detect(&bogus).unwrap();
    assert!(
        missing.is_empty(),
        "v1 detector tolerates missing repo roots without erroring; got {missing:?}",
    );
}
