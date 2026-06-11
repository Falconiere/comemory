// Real small functions copied verbatim from THIS repo:
// src/cli/index_code.rs::line_end_of and src/cli/index_code.rs::relative.
// Both fit comfortably under the chunk line budget, so extraction must
// leave their `chunks` empty. Parsed by tree-sitter, never compiled.
fn line_end_of(s: &ExtractedSymbol) -> usize {
    let lines = s.snippet.lines().count();
    if lines <= 1 {
        s.line
    } else {
        s.line + lines - 1
    }
}

fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .to_string()
}
