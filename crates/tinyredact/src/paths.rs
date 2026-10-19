//! Home-directory path redaction for text that may be persisted or logged.

/// Replace recognized home-directory prefixes with `<HOME>`, preserving the
/// path below the user's home directory and all surrounding text.
///
/// Recognizes `/home/<user>`, `/Users/<user>`, drive-qualified Windows
/// `Users` paths, UNC `Users` and `home` paths, and tilde paths (`~` or
/// `~<user>`). Matching is case-insensitive for the conventional directory
/// names. Other absolute paths are left unchanged.
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
        !previous.is_ascii_alphanumeric() && previous != '_' && previous != '/' && previous != '\\'
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
        return username_end(bytes, start + 9);
    }

    unc_home_end(bytes, start)
}

fn tilde_home_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut end = start + 1;
    while end < bytes.len() && !is_separator(bytes[end]) && !bytes[end].is_ascii_whitespace() {
        end += 1;
    }
    Some(end)
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
    username_end(bytes, root_end + 1)
}

fn segment_end(bytes: &[u8], start: usize) -> Option<usize> {
    let relative_end = bytes[start..].iter().position(|byte| is_separator(*byte))?;
    Some(start + relative_end)
}

fn username_end(bytes: &[u8], start: usize) -> Option<usize> {
    let end = bytes[start..]
        .iter()
        .position(|byte| is_separator(*byte))
        .map_or(bytes.len(), |relative_end| start + relative_end);
    (end > start && !bytes[start..end].iter().any(u8::is_ascii_whitespace)).then_some(end)
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
