// SPDX-License-Identifier: MPL-2.0
//! Bounded launch values for the private bundled frontend wire.

use super::{MAX_PATHS, decode_path, encode_path, require, validate_path_bytes};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchTarget {
    pub path: Vec<u8>,
    pub line: Option<u64>,
    pub column: Option<u64>,
}

impl From<crate::launch::LaunchTarget> for LaunchTarget {
    fn from(target: crate::launch::LaunchTarget) -> Self {
        Self {
            path: encode_path(&target.path),
            line: target.position.map(|p| p.line.get() as u64),
            column: target
                .position
                .and_then(|p| p.column.map(|c| c.get() as u64)),
        }
    }
}

impl TryFrom<LaunchTarget> for crate::launch::LaunchTarget {
    type Error = String;
    fn try_from(target: LaunchTarget) -> Result<Self, Self::Error> {
        target.validate()?;
        let position = target.line.map(|line| crate::launch::LaunchPosition {
            line: usize::try_from(line)
                .ok()
                .and_then(std::num::NonZeroUsize::new)
                .expect("validated line"),
            column: target.column.map(|column| {
                usize::try_from(column)
                    .ok()
                    .and_then(std::num::NonZeroUsize::new)
                    .expect("validated column")
            }),
        });
        Ok(Self {
            path: decode_path(target.path).map_err(|e| e.to_string())?,
            position,
        })
    }
}

impl LaunchTarget {
    fn validate(&self) -> Result<(), String> {
        validate_path_bytes(&self.path)?;
        let path = decode_path(self.path.clone()).map_err(|e| e.to_string())?;
        require(path.is_absolute(), "launch target must be absolute")?;
        require(
            !path.as_os_str().as_encoded_bytes().contains(&0),
            "launch path contains NUL",
        )?;
        require(
            self.column.is_none() || self.line.is_some(),
            "launch column requires a line",
        )?;
        for position in [self.line, self.column].into_iter().flatten() {
            require(
                position > 0 && usize::try_from(position).is_ok(),
                "invalid launch position",
            )?;
        }
        Ok(())
    }
}

pub(super) fn validate_targets(targets: &[LaunchTarget]) -> Result<(), String> {
    require(
        !targets.is_empty() && targets.len() <= MAX_PATHS,
        "launch request requires 1 to 32 targets",
    )?;
    targets.iter().try_for_each(LaunchTarget::validate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::{LaunchPosition, LaunchTarget as CoreTarget};

    #[test]
    fn targets_round_trip_paths_and_caret_without_serializing_core_values() {
        let core = CoreTarget::at(
            std::env::temp_dir().join("文 notes.txt"),
            LaunchPosition {
                line: std::num::NonZeroUsize::new(7).unwrap(),
                column: std::num::NonZeroUsize::new(3),
            },
        );
        let wire = LaunchTarget::from(core.clone());
        let encoded = serde_json::to_vec(&wire).unwrap();
        let decoded: LaunchTarget = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(CoreTarget::try_from(decoded).unwrap(), core);
    }

    #[test]
    fn malformed_or_oversized_launch_requests_fail_before_opening() {
        let valid = LaunchTarget::from(CoreTarget::new(std::env::temp_dir().join("notes.txt")));
        assert!(validate_targets(&vec![valid.clone(); MAX_PATHS]).is_ok());
        assert!(validate_targets(&[]).is_err());
        assert!(validate_targets(&vec![valid.clone(); MAX_PATHS + 1]).is_err());
        let mut bad = valid.clone();
        bad.path.clear();
        assert!(bad.validate().is_err());
        bad = valid.clone();
        bad.path = encode_path(std::path::Path::new("relative"));
        assert!(bad.validate().is_err());
        bad = valid.clone();
        bad.path = vec![b'x'; super::super::MAX_PATH_BYTES + 1];
        assert!(bad.validate().is_err());
        bad = valid.clone();
        bad.line = Some(0);
        assert!(bad.validate().is_err());
        bad = valid.clone();
        bad.column = Some(1);
        assert!(bad.validate().is_err());
        bad = valid;
        bad.line = Some(1);
        bad.column = Some(0);
        assert!(bad.validate().is_err());
    }
}
