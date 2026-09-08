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
    if !web_prefix(text) || text.chars().any(|c| c.is_whitespace() || c.is_control()) {
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
    if host.is_empty() || (bare && host.len() <= 4) {
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
    for (index, _) in chunk.char_indices() {
        let candidate = &chunk[index..];
        if !web_prefix(candidate)
            || (index > 0 && !is_path_boundary(chunk[..index].chars().next_back()?))
        {
            continue;
        }
        let mut link = candidate;
        loop {
            let last = link.chars().next_back()?;
            let unmatched = match last {
                ')' => Some(('(', ')')),
                ']' => Some(('[', ']')),
                '}' => Some(('{', '}')),
                _ => None,
            }
            .is_some_and(|(open, close)| link.matches(close).count() > link.matches(open).count());
            if matches!(last, '.' | ',' | ';' | ':' | '!' | '?') || unmatched {
                link = &link[..link.len() - last.len_utf8()];
            } else {
                break;
            }
        }
        if caret >= start + index && caret < start + index + link.len() && web_url(link).is_some() {
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
    let end = line[caret..]
        .char_indices()
        .find(|(_, c)| is_path_boundary(*c))
        .map_or(line.len(), |(i, _)| caret + i);
    Some(line[start..end].to_owned())
}

#[cfg(test)]
#[path = "navigation_target/tests.rs"]
mod tests;
