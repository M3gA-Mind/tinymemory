//! Home-directory path redaction for text that may be persisted or logged.

use crate::pattern::literal;
use regex::Regex;
use std::sync::LazyLock;

static UNICODE_MARK: LazyLock<Regex> = LazyLock::new(|| literal(r"\A\p{M}\z"));

/// Replace recognized home-directory prefixes with `<HOME>`, preserving the
/// path below the user's home directory and all surrounding text.
///
/// Recognizes `/home/<user>`, `/Users/<user>`, drive-qualified Windows
/// `Users` paths, UNC `Users` and `home` paths, and tilde paths (`~` or
/// `~<user>`). Matching is case-insensitive for the conventional directory
/// names. A username token starts with a Unicode letter or digit (or `_`) and
/// may continue with Unicode marks, letters, digits, `_`, `-`, and `.`;
/// whitespace and other punctuation end it. Trailing dots are kept as
/// surrounding punctuation. A bare `~` is recognized at end of input or
/// before whitespace or a closing delimiter; `~.` and `~.name` are relative
/// paths and are left unchanged. Other absolute paths are left unchanged.
/// Quoted Windows drive and UNC paths may have spaces in the profile component;
/// the matching quote or the next path separator bounds that component.
pub fn redact_home_paths(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = String::with_capacity(text.len());
    let mut position = 0;

    while position < text.len() {
        if has_path_boundary(text, position)
            && let Some(consumed) = home_path_end(bytes, position)
        {
            output.push_str("<HOME>");
            position = consumed;
        } else {
            if let Some(character) = text[position..].chars().next() {
                output.push(character);
                position += character.len_utf8();
            } else {
                break;
            }
        }
    }

    output
}

fn has_path_boundary(text: &str, position: usize) -> bool {
    text[..position].chars().next_back().is_none_or(|previous| {
        !previous.is_alphanumeric() && previous != '_' && previous != '/' && previous != '\\'
    })
}

fn home_path_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes[start] == b'~' {
        return tilde_home_end(bytes, start);
    }

    if starts_with_ascii_case_insensitive(&bytes[start..], b"/home/") {
        return username_end(bytes, start + 6);
    }
    if starts_with_ascii_case_insensitive(&bytes[start..], b"/users/") {
        return username_end(bytes, start + 7);
    }

    if bytes.len() >= start + 9
        && bytes[start].is_ascii_alphabetic()
        && bytes[start + 1] == b':'
        && is_separator(bytes[start + 2])
        && starts_with_ascii_case_insensitive(&bytes[start + 3..start + 8], b"users")
        && is_separator(bytes[start + 8])
    {
        let username_start = start + 9;
        return quoted_profile_end(bytes, start, username_start)
            .or_else(|| username_end(bytes, username_start));
    }

    unc_home_end(bytes, start)
}

fn tilde_home_end(bytes: &[u8], start: usize) -> Option<usize> {
    let after_tilde = start + 1;
    if after_tilde == bytes.len() || is_separator(bytes[after_tilde]) {
        return Some(after_tilde);
    }
    let next = std::str::from_utf8(&bytes[after_tilde..])
        .ok()?
        .chars()
        .next()?;
    if is_username_start(next) {
        return username_end(bytes, after_tilde);
    }
    (next.is_whitespace() || is_closing_delimiter(next)).then_some(after_tilde)
}

fn unc_home_end(bytes: &[u8], start: usize) -> Option<usize> {
    if start + 2 > bytes.len() || !is_separator(bytes[start]) || !is_separator(bytes[start + 1]) {
        return None;
    }
    let server_end = segment_end(bytes, start + 2)?;
    if server_end == start + 2 || server_end == bytes.len() {
        return None;
    }
    let root_start = server_end + 1;
    let root_end = segment_end(bytes, root_start)?;
    if root_end == root_start || root_end == bytes.len() {
        return None;
    }
    let root = &bytes[root_start..root_end];
    if !root.eq_ignore_ascii_case(b"users") && !root.eq_ignore_ascii_case(b"home") {
        return None;
    }
    let username_start = root_end + 1;
    quoted_profile_end(bytes, start, username_start).or_else(|| username_end(bytes, username_start))
}

fn segment_end(bytes: &[u8], start: usize) -> Option<usize> {
    let relative_end = bytes[start..].iter().position(|byte| is_separator(*byte))?;
    Some(start + relative_end)
}

fn username_end(bytes: &[u8], start: usize) -> Option<usize> {
    let text = std::str::from_utf8(&bytes[start..]).ok()?;
    let mut end = start;
    for (offset, character) in text.char_indices() {
        if (offset == 0 && !is_username_start(character)) || !is_username_character(character) {
            break;
        }
        end = start + offset + character.len_utf8();
    }

    // A final period is usually sentence punctuation rather than part of the
    // account name. Interior dots remain valid (`alice.smith`).
    while end > start && bytes[end - 1] == b'.' {
        end -= 1;
    }
    (end > start).then_some(end)
}

/// Recognize a space-containing Windows profile component only when its path
/// starts inside a matching quoted string. The next separator marks the end of
/// a profile with descendants; otherwise the closing quote marks a bare home.
/// Unquoted text keeps the normal single-token rule so prose is never consumed.
fn quoted_profile_end(bytes: &[u8], path_start: usize, username_start: usize) -> Option<usize> {
    let quote = *bytes.get(path_start.checked_sub(1)?)?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }

    let quote_end = bytes[username_start..]
        .iter()
        .position(|byte| *byte == quote)
        .map(|offset| username_start + offset)?;
    if quote_end == username_start {
        return None;
    }

    let component_end = bytes[username_start..quote_end]
        .iter()
        .position(|byte| is_separator(*byte))
        .map_or(quote_end, |offset| username_start + offset);
    let component = std::str::from_utf8(&bytes[username_start..component_end]).ok()?;
    if !component.chars().any(char::is_whitespace) {
        return None;
    }

    Some(component_end)
}

fn is_username_start(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn is_username_character(character: char) -> bool {
    is_username_start(character)
        || character == '-'
        || character == '.'
        || is_unicode_mark(character)
}

fn is_unicode_mark(character: char) -> bool {
    let mut encoded = [0; 4];
    UNICODE_MARK.is_match(character.encode_utf8(&mut encoded))
}

fn is_closing_delimiter(character: char) -> bool {
    matches!(character, ')' | ']' | '}' | '\'' | '"' | '`' | '>')
}

fn is_separator(byte: u8) -> bool {
    byte == b'/' || byte == b'\\'
}

fn starts_with_ascii_case_insensitive(value: &[u8], prefix: &[u8]) -> bool {
    value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix)
}

#[cfg(test)]
#[path = "paths_tests.rs"]
mod tests;
