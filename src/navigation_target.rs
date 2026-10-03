// SPDX-License-Identifier: MPL-2.0

//! File paths and visible web links in document or terminal review text.

pub(crate) fn is_path_boundary(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '"' | '\'' | '`' | '<' | '>' | '|' | ';' | ',' | '(' | ')' | '[' | ']' | '{' | '}'
        )
}

fn web_prefix(text: &str) -> bool {
    text.get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
        || text
            .get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case("http://"))
        || text
            .get(..4)
            .is_some_and(|s| s.eq_ignore_ascii_case("www."))
}

/// Only explicit web addresses are handed to the desktop opener. Bare www
/// addresses use HTTPS; selected targets otherwise retain their exact spelling.
pub(crate) fn web_url(text: &str) -> Option<String> {
    if !web_prefix(text) {
        return None;
    }
    let bare = text
        .get(..4)
        .is_some_and(|s| s.eq_ignore_ascii_case("www."));
    let authority = if bare {
        text
    } else {
        text.split_once("://")?.1
    };
    let host = authority.split(['/', '?', '#']).next()?;
    if host.is_empty()
        || (bare && host.len() <= 4)
        || text.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    Some(if bare {
        format!("https://{text}")
    } else {
        text.to_owned()
    })
}

/// Finds a target at a character offset within one row. Web links may contain
/// path-token delimiters such as commas and balanced parentheses. Surrounding
/// prose punctuation and Markdown wrappers are excluded from inferred links.
pub(crate) fn under_cursor(line: &str, offset: usize) -> Option<String> {
    let caret = line.char_indices().nth(offset)?.0;
    let hard_boundary = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>');
    let start = line[..caret]
        .char_indices()
        .rev()
        .find(|(_, c)| hard_boundary(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = line[caret..]
        .char_indices()
        .find(|(_, c)| hard_boundary(*c))
        .map_or(line.len(), |(i, _)| caret + i);
    let chunk = &line[start..end];
    // Count delimiters once for the whole token. As candidate prefixes are
    // visited, subtract the characters to their left. The trailing run can
    // then be trimmed with three indexed lookups, including when a caret on
    // excluded punctuation makes us try many URL prefixes in the same token.
    let mut opens = [0usize; 3];
    let mut closes = [0usize; 3];
    let mut last_control = None;
    for (index, character) in chunk.char_indices() {
        if character.is_control() {
            last_control = Some(index);
        }
        match character {
            '(' => opens[0] += 1,
            '[' => opens[1] += 1,
            '{' => opens[2] += 1,
            ')' => closes[0] += 1,
            ']' => closes[1] += 1,
            '}' => closes[2] += 1,
            _ => {}
        }
    }
    let mut suffix_closers = [Vec::new(), Vec::new(), Vec::new()];
    let mut suffix_start = chunk.len();
    for (index, character) in chunk.char_indices().rev() {
        match character {
            ')' => suffix_closers[0].push(index),
            ']' => suffix_closers[1].push(index),
            '}' => suffix_closers[2].push(index),
            '.' | ',' | ';' | ':' | '!' | '?' => {}
            _ => break,
        }
        suffix_start = index;
    }
    for (index, character) in chunk.char_indices() {
        match character {
            '(' => opens[0] -= 1,
            '[' => opens[1] -= 1,
            '{' => opens[2] -= 1,
            ')' => closes[0] -= 1,
            ']' => closes[1] -= 1,
            '}' => closes[2] -= 1,
            _ => {}
        }
        let candidate = &chunk[index..];
        if !web_prefix(candidate)
            || (index > 0 && !is_path_boundary(chunk[..index].chars().next_back()?))
        {
            continue;
        }
        // A closer can be discarded only while that kind has more closing
        // than opening delimiters. The first closer beyond that excess stops
        // trimming; the rightmost such stop wins across the three kinds.
        let mut link_end = suffix_start;
        for kind in 0..3 {
            let excess = closes[kind].saturating_sub(opens[kind]);
            if let Some(&position) = suffix_closers[kind].get(excess) {
                link_end = link_end.max(position + 1);
            }
        }
        let link = &chunk[index..link_end];
        if caret >= start + index
            && caret < start + index + link.len()
            && last_control.is_none_or(|control| control < index)
            && web_url(link).is_some()
        {
            return Some(link.to_owned());
        }
    }
    if is_path_boundary(line[caret..].chars().next()?) {
        return None;
    }
    let start = line[..caret]
        .char_indices()
        .rev()
        .find(|(_, c)| is_path_boundary(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let mut end = line[caret..]
        .char_indices()
        .find(|(_, c)| is_path_boundary(*c))
        .map_or(line.len(), |(i, _)| caret + i);
    // Keep punctuation that closes a token at the end of prose so path
    // resolution can prefer a real filename containing it. An internal
    // delimiter, as in `one,two`, still separates the two tokens.
    let suffix = &line[end..];
    let punctuation_bytes = suffix
        .chars()
        .take_while(|c| matches!(c, '.' | ',' | ':' | ';' | '!' | '?' | ')' | ']' | '}'))
        .map(char::len_utf8)
        .sum::<usize>();
    if punctuation_bytes > 0
        && suffix[punctuation_bytes..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || hard_boundary(c))
    {
        end += punctuation_bytes;
    }
    Some(line[start..end].to_owned())
}

/// Decodes `%XX` escapes, leaving malformed ones and invalid UTF-8 as written.
pub(crate) fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = (bytes[index] == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = escape {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| text.to_owned())
}

#[cfg(test)]
#[path = "navigation_target/tests.rs"]
mod tests;
