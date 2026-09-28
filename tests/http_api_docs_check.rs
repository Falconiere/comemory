#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Gate: every live `/api/v1` route in [`comemory::serve::routes::table`]
//! must appear in `docs/guides/http-api.md`'s `## Route map` section. The
//! doc is hand-authored (its own header says so), so this is a presence
//! check, not a byte-for-byte regen diff like `cli-docs-check.sh`.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use comemory::serve::routes;
use regex::Regex;

const HTTP_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE"];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The `## Route map` section text: from that heading (exclusive) to the
/// next level-2 (`## `) heading (exclusive), or end of file if none follows.
/// Bounded by heading text rather than a line number so the doc can grow
/// without this test needing an update.
fn route_map_section(doc: &str) -> &str {
    let start = doc
        .find("## Route map")
        .expect("docs/guides/http-api.md has no '## Route map' heading");
    let after_heading = &doc[start..];
    let body_start = after_heading
        .find('\n')
        .map_or(after_heading.len(), |i| i + 1);
    let body = &after_heading[body_start..];
    match body[..].find("\n## ") {
        Some(end) => &body[..end],
        None => body,
    }
}

/// Every `(method, path)` pair documented inside `section`'s backtick spans.
/// A span counts only when it starts with one or more `|`/`\|`-joined HTTP
/// methods followed by whitespace and a path — a bare `` `GET` `` (used in
/// prose, e.g. "`GET` endpoints take query params") has no trailing path and
/// is correctly ignored, never counted as a route. A `?query` suffix is
/// stripped before comparison since `RouteEntry::path` never carries one.
fn extract_documented_routes(section: &str) -> HashSet<(String, String)> {
    let span_re = Regex::new(r"`([^`]+)`").unwrap();
    let methods_alt = HTTP_METHODS.join("|");
    let route_re = Regex::new(&format!(
        r"^((?:{methods_alt})(?:\\?\|(?:{methods_alt}))*)\s+(/\S+)$"
    ))
    .unwrap();

    let mut routes = HashSet::new();
    for span in span_re.captures_iter(section) {
        let content = &span[1];
        let Some(caps) = route_re.captures(content) else {
            continue;
        };
        let methods = &caps[1];
        let path = caps[2].split('?').next().unwrap_or(&caps[2]);
        for method in methods.split("\\|").flat_map(|m| m.split('|')) {
            routes.insert((method.to_string(), path.to_string()));
        }
    }
    routes
}

#[test]
fn extract_documented_routes_handles_every_doc_format_variant() {
    let single = extract_documented_routes("○ `GET /memories`");
    assert_eq!(
        single,
        HashSet::from([("GET".to_string(), "/memories".to_string())])
    );

    let multi_method = extract_documented_routes(r"○ `GET\|POST /memories/search`");
    assert_eq!(
        multi_method,
        HashSet::from([
            ("GET".to_string(), "/memories/search".to_string()),
            ("POST".to_string(), "/memories/search".to_string()),
        ])
    );

    let with_query = extract_documented_routes("● `DELETE /memories/{id}?confirm=true`");
    assert_eq!(
        with_query,
        HashSet::from([("DELETE".to_string(), "/memories/{id}".to_string())])
    );

    let path_param = extract_documented_routes("● `PUT /hooks/{name}?repo=`");
    assert_eq!(
        path_param,
        HashSet::from([("PUT".to_string(), "/hooks/{name}".to_string())])
    );

    // A bare method with no trailing path (prose, not a route) must not
    // produce a spurious match.
    let prose = extract_documented_routes(
        "`GET` endpoints take query params; `POST` endpoints take a JSON body",
    );
    assert!(
        prose.is_empty(),
        "bare-method prose must not be parsed as a route: {prose:?}"
    );
}

#[test]
fn http_api_guide_documents_every_live_route() {
    let doc_path = root().join("docs/guides/http-api.md");
    let doc = fs::read_to_string(&doc_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", doc_path.display()));
    let documented = extract_documented_routes(route_map_section(&doc));

    let live = routes::table();
    assert!(
        !live.is_empty(),
        "comemory::serve::routes::table() is empty"
    );

    let missing: Vec<String> = live
        .iter()
        .filter(|r| !documented.contains(&(r.method.to_string(), r.path.to_string())))
        .map(|r| format!("{} {}", r.method, r.path))
        .collect();

    assert!(
        missing.is_empty(),
        "docs/guides/http-api.md's Route map is missing {} route(s):\n{}\n\
         Add each missing route to its resource's table in the Route map section.",
        missing.len(),
        missing.join("\n")
    );
}
