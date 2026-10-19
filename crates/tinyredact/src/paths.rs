//! Home-directory path redaction for text that may be persisted or logged.

/// Replace recognized home-directory prefixes with `<HOME>`, preserving the
/// path below the user's home directory and all surrounding text.
///
/// Recognizes `/home/<user>`, `/Users/<user>`, drive-qualified Windows
/// `Users` paths, UNC `Users` and `home` paths, and tilde paths (`~` or
/// `~<user>`). Matching is case-insensitive for the conventional directory
/// names. After one of these explicit home prefixes, the nonempty profile
/// component is redacted through its path separator, including arbitrary UTF-8
/// and symbols. When no descendant separator is present, whitespace and clear
/// closing punctuation end the component; terminal dots, commas and semicolons
/// remain surrounding text. A bare `~` is recognized at end of input or
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
    if let Some((offset, character)) = text
        .char_indices()
        .find(|(_, character)| character.is_whitespace() || matches!(character, '/' | '\\'))
    {
        if matches!(character, '/' | '\\') {
            return (offset > 0).then_some(start + offset);
        }
    }

    let mut end = start;
    for (offset, character) in text.char_indices() {
        if matches!(character, '/' | '\\') || character.is_whitespace() {
            break;
        }
        if is_bare_component_boundary(character, text, offset) {
            break;
        }
        end = start + offset + character.len_utf8();
    }

    // Terminal punctuation belongs to surrounding text; interior symbols
    // remain part of the profile component.
    while end > start && matches!(bytes[end - 1], b'.' | b',' | b';') {
        end -= 1;
    }
    (end > start).then_some(end)
}

fn is_bare_component_boundary(character: char, text: &str, offset: usize) -> bool {
    if matches!(character, ')' | ']' | '}' | '>' | ',' | ';') {
        return true;
    }
    if character == '.' {
        let after = offset + character.len_utf8();
        return text[after..].chars().next().is_none_or(|next| {
            next.is_whitespace() || matches!(next, '/' | '\\' | ',' | ';' | ')' | ']' | '}')
        });
    }
    if matches!(character, '!' | '?') {
        let after = offset + character.len_utf8();
        return text[after..]
            .chars()
            .next()
            .is_none_or(|next| next.is_whitespace() || matches!(next, ')' | ']' | '}'));
    }
    if matches!(character, '\'' | '"') {
        let after = offset + character.len_utf8();
        return text[after..].chars().next().is_none_or(|next| {
            next.is_whitespace() || matches!(next, ',' | ';' | ')' | ']' | '}')
        });
    }
    false
}

/// Use an enclosing quote or the next path separator to bound a Windows
/// profile. A quoted bare profile ends at the matching quote; a profile with
/// descendants ends at the first separator. Quoted names may contain spaces
/// and symbols without consuming following prose.
fn quoted_profile_end(bytes: &[u8], path_start: usize, username_start: usize) -> Option<usize> {
    let quote = *bytes.get(path_start.checked_sub(1)?)?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }

    let suffix = std::str::from_utf8(&bytes[username_start..]).ok()?;
    for (offset, character) in suffix.char_indices() {
        if matches!(character, '/' | '\\') {
            return (offset > 0).then_some(username_start + offset);
        }
        if character as u32 == quote as u32 {
            let after = offset + character.len_utf8();
            if suffix[after..].chars().next().is_none_or(|next| {
                next.is_whitespace() || matches!(next, ',' | ';' | ')' | ']' | '}')
            }) {
                return (offset > 0).then_some(username_start + offset);
            }
        }
    }
    None
}

fn is_username_start(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
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
