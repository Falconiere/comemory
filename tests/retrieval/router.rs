use comemory::retrieval::router::{classify, Route};

#[test]
fn symbol_looking_query_routes_to_symbol() {
    assert_eq!(classify("handleLogin"), Route::Symbol);
    assert_eq!(classify("run_migration"), Route::Symbol);
}

#[test]
fn long_question_routes_to_hybrid() {
    assert_eq!(classify("postgres migration race condition"), Route::Hybrid);
}

#[test]
fn empty_query_routes_to_fts_first() {
    assert_eq!(classify(""), Route::FtsFirst);
}

#[test]
fn whitespace_only_query_routes_to_fts_first() {
    assert_eq!(classify("   \t  "), Route::FtsFirst);
}

#[test]
fn two_word_query_routes_to_fts_first() {
    assert_eq!(classify("postgres migration"), Route::FtsFirst);
}

#[test]
fn single_token_with_punct_is_not_symbol() {
    // A trailing `?` is not in the identifier charset, so this falls through
    // to the FtsFirst branch (single token, but not symbol-like).
    assert_eq!(classify("login?"), Route::FtsFirst);
}
