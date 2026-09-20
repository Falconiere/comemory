//! Finding the model in whatever an agent printed. Agents wrap JSON in prose,
//! in a fence, or in nothing at all; all three are read the same way, and a
//! fourth case — no JSON anywhere — falls through to the parser so the failure
//! is a real parse error rather than a guess.

/// The best model-JSON candidate in `stdout`: the first ```json fence, else
/// the first balanced top-level object, else the whole text.
pub fn model_json(stdout: &str) -> &str {
    // The fence search is inline rather than its own helper: as a sibling
    // function it is a near-duplicate of `balanced` by shape (find, offset,
    // slice), which `scripts/dup-check.sh` counts.
    let fenced = stdout.find("```json").and_then(|open| {
        let rest = &stdout[open + "```json".len()..];
        let start = rest.find('\n')? + 1;
        let end = rest[start..].find("```")? + start;
        Some(&rest[start..end])
    });
    fenced.or_else(|| balanced(stdout)).unwrap_or(stdout)
}

/// The first balanced `{ … }` region, honouring strings and escapes so a brace
/// inside a summary does not end the object early.
fn balanced(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, c) in text[start..].char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
#[path = "tests/extract.rs"]
mod tests;
