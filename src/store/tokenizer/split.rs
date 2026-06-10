//! Splits text into FTS tokens with camelCase / snake_case / digit
//! boundary awareness. Each alphanumeric run yields its sub-parts at
//! distinct token positions; when a run splits into more than one part,
//! the whole lowercased run is also emitted colocated with the first
//! part so exact-identifier queries still match. Emitted token text is
//! lowercased and diacritic-folded (`café` → `cafe`), restoring the
//! `remove_diacritics 2` behavior of the unicode61 tokenizer this one
//! replaced — both the index and the query side go through this
//! tokenizer, so the folding is symmetric.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// One token produced by [`split_text`]: lowercased text, byte range in
/// the original input, and whether FTS5 should treat it as colocated
/// with the previous token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitToken {
    /// Lowercased token text.
    pub text: String,
    /// Byte offset of the token start in the original text.
    pub start: usize,
    /// Byte offset of the token end in the original text.
    pub end: usize,
    /// True when FTS5 should place this token at the previous token's position.
    pub colocated: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Class {
    Lower,
    Upper,
    Digit,
    Other,
}

fn classify(c: char) -> Class {
    if c.is_lowercase() {
        Class::Lower
    } else if c.is_uppercase() {
        Class::Upper
    } else if c.is_ascii_digit() {
        Class::Digit
    } else {
        Class::Other
    }
}

/// Tokenize `text` into identifier-aware [`SplitToken`]s.
///
/// Alphanumeric-plus-underscore runs are split at camelCase transitions,
/// letter↔digit transitions, and `_` separators. When a run yields more
/// than one part the whole lowercased run is emitted colocated with the
/// first part so exact-identifier queries still match.
pub fn split_text(text: &str) -> Vec<SplitToken> {
    let mut out = Vec::new();
    let mut run_start: Option<usize> = None;
    let mut iter = text.char_indices().peekable();
    while let Some(&(i, c)) = iter.peek() {
        let is_word = c.is_alphanumeric() || c == '_';
        match (run_start, is_word) {
            (None, true) => {
                run_start = Some(i);
                iter.next();
            }
            (Some(s), false) => {
                emit_run(text, s, i, &mut out);
                run_start = None;
                iter.next();
            }
            _ => {
                iter.next();
            }
        }
    }
    if let Some(s) = run_start {
        emit_run(text, s, text.len(), &mut out);
    }
    out
}

/// True when lowercasing left no uppercase char behind. A handful of
/// Unicode codepoints (e.g. `🄰` U+1F130) report `is_uppercase()` but
/// have no lowercase mapping; tokens containing them are dropped so
/// every emitted token is genuinely lowercase.
fn fully_lowercased(token: &str) -> bool {
    !token.chars().any(char::is_uppercase)
}

/// Strip diacritics: NFD-decompose and drop combining marks, so `café` /
/// `naïve` / `über` fold to `cafe` / `naive` / `uber`. Restores the
/// recall the old `unicode61 remove_diacritics 2` tokenizer provided.
/// Exposed so the tests can express the index↔query symmetry invariant.
pub fn fold_diacritics(s: &str) -> String {
    s.nfd().filter(|c| !is_combining_mark(*c)).collect()
}

/// Normalize raw token text for emission: lowercase, then diacritic-fold.
fn normalize_token(raw: &str) -> String {
    fold_diacritics(&raw.to_lowercase())
}

fn emit_run(text: &str, s: usize, e: usize, out: &mut Vec<SplitToken>) {
    let run = &text[s..e];
    let parts = part_ranges(run);
    if parts.is_empty() {
        return;
    }
    if let [(ps, pe)] = parts[..] {
        let part = normalize_token(&run[ps..pe]);
        if !part.is_empty() && fully_lowercased(&part) {
            out.push(SplitToken {
                text: part,
                start: s + ps,
                end: s + pe,
                colocated: false,
            });
        }
        return;
    }
    let whole = normalize_token(run);
    let whole_ok = fully_lowercased(&whole);
    let mut first = true;
    for (ps, pe) in parts {
        let part = normalize_token(&run[ps..pe]);
        if part.is_empty() || !fully_lowercased(&part) {
            continue;
        }
        out.push(SplitToken {
            text: part,
            start: s + ps,
            end: s + pe,
            colocated: false,
        });
        if first && whole_ok {
            out.push(SplitToken {
                text: whole.clone(),
                start: s,
                end: e,
                colocated: true,
            });
        }
        first = false;
    }
}

/// Byte ranges of the sub-parts inside one alphanumeric+`_` run.
///
/// A new part starts at each of three boundaries:
/// 1. `_` separators — underscores terminate the current part and are
///    never included in any part;
/// 2. lower→upper transitions (`camelCase`), plus the last uppercase of
///    an acronym run followed by lowercase (`HTMLParser` → `html` +
///    `parser`);
/// 3. digit boundaries — any digit↔non-digit transition (`sha256sum` →
///    `sha` + `256` + `sum`).
fn part_ranges(run: &str) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = run.char_indices().collect();
    let mut ranges = Vec::new();
    let mut start: Option<usize> = None;
    for w in 0..chars.len() {
        let (i, c) = chars[w];
        if c == '_' {
            if let Some(s) = start.take() {
                ranges.push((s, i));
            }
            continue;
        }
        let cls = classify(c);
        if let Some(s) = start {
            let prev = classify(chars[w - 1].1);
            let lower_to_upper = prev == Class::Lower && cls == Class::Upper;
            let digit_boundary = (prev == Class::Digit) != (cls == Class::Digit);
            let upper_run_ends = prev == Class::Upper
                && cls == Class::Lower
                && w >= 2
                && classify(chars[w - 2].1) == Class::Upper;
            if lower_to_upper || digit_boundary {
                ranges.push((s, i));
                start = Some(i);
            } else if upper_run_ends {
                let prev_i = chars[w - 1].0;
                if prev_i > s {
                    ranges.push((s, prev_i));
                    start = Some(prev_i);
                }
            }
        } else {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        ranges.push((s, run.len()));
    }
    ranges
}
