use qwick_memory::graph::cross_link::extract_refs;

#[test]
fn extracts_file_and_symbol_refs() {
    let body = "See qwick-backend:src/db.rs:run_migration for the call; \
        also touches qwick-backend:src/util.rs.";
    let r = extract_refs(body);
    assert!(
        r.files.contains(&"qwick-backend:src/db.rs".to_string()),
        "expected qwick-backend:src/db.rs in files, got {:?}",
        r.files,
    );
    assert!(
        r.files.contains(&"qwick-backend:src/util.rs".to_string()),
        "expected qwick-backend:src/util.rs in files, got {:?}",
        r.files,
    );
    assert!(
        r.symbols
            .contains(&"qwick-backend:src/db.rs:run_migration".to_string()),
        "expected qwick-backend:src/db.rs:run_migration in symbols, got {:?}",
        r.symbols,
    );
}

#[test]
fn symbol_match_still_yields_file_ref() {
    // A symbol mention must also register the parent file so a memory that
    // names only `<repo>:<path>:<sym>` still gets a ReferencesFile edge.
    let r = extract_refs("touches qwick-frontend:src/app.ts:render");
    assert_eq!(
        r.files,
        vec!["qwick-frontend:src/app.ts".to_string()],
        "file ref should be derived from symbol mention",
    );
    assert_eq!(
        r.symbols,
        vec!["qwick-frontend:src/app.ts:render".to_string()],
    );
}

#[test]
fn duplicate_mentions_are_collapsed() {
    let body = "First qwick-backend:src/db.rs and again qwick-backend:src/db.rs.";
    let r = extract_refs(body);
    assert_eq!(
        r.files.len(),
        1,
        "duplicate file mention must collapse, got {:?}",
        r.files,
    );
    assert!(
        r.symbols.is_empty(),
        "no symbol expected, got {:?}",
        r.symbols
    );
}

#[test]
fn body_without_refs_returns_empty() {
    let r = extract_refs("Just a plain memory body with no code references.");
    assert!(r.files.is_empty());
    assert!(r.symbols.is_empty());
}

#[test]
fn requires_extension_to_match() {
    // Without an extension on the path, the regex must not match — guards
    // against false positives like `org:project`.
    let r = extract_refs("see qwick-backend:src/db for the call");
    assert!(r.files.is_empty(), "no extension should mean no match");
    assert!(r.symbols.is_empty());
}

#[test]
fn ignores_url_like_matches() {
    // Prose memories often include URLs to source files. The regex would
    // otherwise capture `https:` as a repo and `//github.com/.../bar.rs` as
    // a path; the scp-style git URL `git@host:foo/bar.rs:fn` would yield a
    // symbol ref. The URL filter (post-extraction) MUST drop both shapes.
    let body = "see https://github.com/foo/bar.rs and git@github.com:foo/bar.rs:fn for details";
    let r = extract_refs(body);
    assert!(
        r.files.is_empty(),
        "URL-like matches must not produce file refs, got {:?}",
        r.files,
    );
    assert!(
        r.symbols.is_empty(),
        "URL-like matches must not produce symbol refs, got {:?}",
        r.symbols,
    );
}
