// SPDX-License-Identifier: MPL-2.0

//! Pure, conservative parsing of standard, diff3 and zdiff3 conflict regions.
//! Offsets count characters, matching editor transactions. Index stages remain
//! the authority for whether the file is unresolved.

use std::ops::Range;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictRegion {
    pub identity: String,
    pub range: Range<usize>,
    pub current: Range<usize>,
    pub base: Option<Range<usize>>,
    pub other: Range<usize>,
}

pub fn parse_conflict_regions(
    text: &str,
    width: usize,
) -> Result<Vec<ConflictRegion>, &'static str> {
    if !(1..=1024).contains(&width) {
        return Err("unsupported conflict marker width");
    }
    let mut regions = Vec::new();
    let mut offset = 0;
    let mut start = None;
    let mut current_start = 0;
    let mut current_end = None;
    let mut base = None;
    let mut other_start = None;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let next = offset + line.chars().count();
        let marker = ['<', '|', '=', '>'].into_iter().find(|&marker| {
            let mut chars = body.chars();
            (0..width).all(|_| chars.next() == Some(marker))
                && matches!(chars.next(), None | Some(' '))
        });
        match marker {
            Some('<') if start.is_none() => {
                start = Some(offset);
                current_start = next;
                current_end = None;
                base = None;
                other_start = None;
            }
            Some('|') if start.is_some() && current_end.is_none() && other_start.is_none() => {
                current_end = Some(offset);
                base = Some(next..next);
            }
            Some('=') if start.is_some() && other_start.is_none() => {
                if let Some(range) = base.as_mut() {
                    range.end = offset;
                } else {
                    current_end = Some(offset);
                }
                other_start = Some(next);
            }
            Some('>') if start.is_some() && other_start.is_some() => {
                let range = start.take().unwrap()..next;
                let content: String = text.chars().skip(range.start).take(range.len()).collect();
                regions.push(ConflictRegion {
                    identity: crate::hash::sha256_hex(
                        format!("{}:{content}", range.start).as_bytes(),
                    ),
                    range,
                    current: current_start..current_end.unwrap(),
                    base: base.take(),
                    other: other_start.take().unwrap()..offset,
                });
            }
            Some(_) => return Err("ambiguous or malformed conflict markers"),
            None => (),
        }
        offset = next;
    }
    if start.is_some() {
        return Err("unterminated conflict region");
    }
    Ok(regions)
}
