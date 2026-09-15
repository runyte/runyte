// SPDX-License-Identifier: MPL-2.0

//! Bounded release intervals, independent of configuration and process IO.

use anyhow::{Result, bail, ensure};
use std::collections::BTreeSet;

pub const MAX_RANGE_BYTES: usize = 256;
pub const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const STABLE_VERSION: &str = "0.3.0";
pub const STABLE_RANGE: &str = ">=0.3.0, <0.4.0";

pub fn negotiate_features(
    required: &BTreeSet<String>,
    optional: &BTreeSet<String>,
    supported: &[&str],
) -> Result<BTreeSet<String>, String> {
    if required.len() + optional.len() > 32
        || !required.is_disjoint(optional)
        || required.union(optional).any(|name| {
            name.is_empty()
                || name.len() > 64
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_'))
        })
    {
        return Err("Invalid feature declarations".into());
    }
    if let Some(name) = required
        .iter()
        .find(|name| !supported.contains(&name.as_str()))
    {
        return Err(format!("Host does not support required feature {name}"));
    }
    Ok(required
        .union(optional)
        .filter(|name| supported.contains(&name.as_str()))
        .cloned()
        .collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    prerelease: Option<String>,
}

impl Version {
    pub fn parse(value: &str) -> Result<Self> {
        ensure!(
            !value.is_empty() && value.len() <= MAX_RANGE_BYTES,
            "invalid version length"
        );
        let (version, build) = value
            .split_once('+')
            .map_or((value, None), |(v, b)| (v, Some(b)));
        if let Some(build) = build {
            identifiers(build, false)?;
        }
        let (core, prerelease) = version
            .split_once('-')
            .map_or((version, None), |(v, p)| (v, Some(p)));
        if let Some(prerelease) = prerelease {
            identifiers(prerelease, true)?;
        }
        let parts: Vec<_> = core.split('.').collect();
        ensure!(parts.len() == 3, "version needs three numeric components");
        let mut core = [0; 3];
        for (target, part) in core.iter_mut().zip(parts) {
            ensure!(
                !part.is_empty()
                    && part.bytes().all(|c| c.is_ascii_digit())
                    && (part == "0" || !part.starts_with('0')),
                "invalid numeric version component"
            );
            *target = part
                .parse()
                .map_err(|_| anyhow::anyhow!("version component overflow"))?;
        }
        Ok(Self {
            core,
            prerelease: prerelease.map(str::to_owned),
        })
    }
}

fn identifiers(value: &str, prerelease: bool) -> Result<()> {
    for part in value.split('.') {
        ensure!(
            !part.is_empty() && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "invalid version identifier"
        );
        ensure!(
            !prerelease
                || !part.bytes().all(|c| c.is_ascii_digit())
                || part == "0"
                || !part.starts_with('0'),
            "leading zero in prerelease identifier"
        );
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct ReleaseRange {
    first: [u64; 3],
    last: [u64; 3],
    prerelease: Option<String>,
    normalized: String,
}

fn adjacent(mut core: [u64; 3], forward: bool) -> Option<[u64; 3]> {
    for component in core.iter_mut().rev() {
        if let Some(value) = if forward {
            component.checked_add(1)
        } else {
            component.checked_sub(1)
        } {
            *component = value;
            return Some(core);
        }
        *component = if forward { 0 } else { u64::MAX };
    }
    None
}

impl ReleaseRange {
    pub fn parse(value: &str) -> Result<Self> {
        ensure!(
            !value.is_empty() && value.len() <= MAX_RANGE_BYTES,
            "range must contain 1–256 bytes"
        );
        ensure!(
            !value.contains('+')
                && !value
                    .chars()
                    .any(|c| c.is_whitespace() && !matches!(c, ' ' | '\t')),
            "invalid range whitespace or build metadata"
        );
        let value = value.trim_matches([' ', '\t']);
        if let Some(exact) = value.strip_prefix('=') {
            let exact = exact.trim_matches([' ', '\t']);
            let version = Version::parse(exact)?;
            return Ok(Self {
                first: version.core,
                last: version.core,
                prerelease: version.prerelease,
                normalized: format!("={exact}"),
            });
        }
        let terms: Vec<_> = value.split(',').collect();
        ensure!(
            terms.len() == 2,
            "range needs one lower and one upper bound, or =version"
        );
        let mut lower = None;
        let mut upper = None;
        for term in terms {
            let term = term.trim_matches([' ', '\t']);
            let (operator, version) = [">=", "<=", ">", "<"]
                .into_iter()
                .find_map(|op| {
                    term.strip_prefix(op)
                        .map(|v| (op, v.trim_matches([' ', '\t'])))
                })
                .ok_or_else(|| anyhow::anyhow!("unsupported range comparator"))?;
            let parsed = Version::parse(version)?;
            ensure!(
                parsed.prerelease.is_none(),
                "prereleases require an exact =version range"
            );
            let normalized = format!("{operator}{version}");
            if operator.starts_with('>') {
                ensure!(lower.is_none(), "duplicate lower bound");
                let core = if operator == ">" {
                    adjacent(parsed.core, true).ok_or_else(|| anyhow::anyhow!("empty range"))?
                } else {
                    parsed.core
                };
                lower = Some((core, normalized));
            } else {
                ensure!(upper.is_none(), "duplicate upper bound");
                let core = if operator == "<" {
                    adjacent(parsed.core, false).ok_or_else(|| anyhow::anyhow!("empty range"))?
                } else {
                    parsed.core
                };
                upper = Some((core, normalized));
            }
        }
        let (Some((first, lower)), Some((last, upper))) = (lower, upper) else {
            bail!("range needs a lower and upper bound");
        };
        ensure!(first <= last, "empty range");
        Ok(Self {
            first,
            last,
            prerelease: None,
            normalized: format!("{lower}, {upper}"),
        })
    }

    pub fn contains(&self, version: &Version) -> bool {
        self.prerelease == version.prerelease
            && self.first <= version.core
            && version.core <= self.last
    }

    pub fn is_subset_of(&self, authored: &Self) -> bool {
        self.prerelease == authored.prerelease
            && self.first >= authored.first
            && self.last <= authored.last
    }

    pub fn normalized(&self) -> &str {
        &self.normalized
    }
}

#[cfg(test)]
#[path = "tests/compatibility.rs"]
mod tests;
