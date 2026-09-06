// SPDX-License-Identifier: MPL-2.0

//! Private integrated-terminal launch context. A marker identifies a candidate
//! parent; the host also verifies the live terminal and the socket peer's Unix
//! session before it accepts a request.

use crate::{hash::sha256_hex, terminal::TerminalId};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub const ENVIRONMENT: &str = "RUNYTE_PARENT_CONTEXT";

#[derive(Clone)]
pub struct ParentLaunch {
    metadata: PathBuf,
    secret: String,
}

impl std::fmt::Debug for ParentLaunch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParentLaunch")
            .field("metadata", &self.metadata)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParentContext {
    pub protocol: u32,
    pub metadata: Vec<u8>,
    pub terminal: u64,
    pub capability: String,
}

impl ParentLaunch {
    pub fn new(metadata: &Path) -> Result<Self> {
        let mut random = [0; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        Ok(Self {
            metadata: metadata.to_owned(),
            secret: sha256_hex(&random),
        })
    }

    fn capability(&self, id: TerminalId) -> String {
        sha256_hex(format!("{}:{}", self.secret, id.get()).as_bytes())
    }

    pub fn context(&self, id: TerminalId) -> String {
        serde_json::to_string(&ParentContext {
            protocol: crate::protocol::VERSION,
            metadata: crate::protocol::encode_path(&self.metadata),
            terminal: id.get(),
            capability: self.capability(id),
        })
        .expect("parent launch context contains serializable scalar fields")
    }

    pub fn validates(&self, terminal: TerminalId, capability: &str) -> bool {
        self.capability(terminal) == capability
    }
}

impl ParentContext {
    pub fn from_environment() -> Result<Option<Self>> {
        let Some(value) = std::env::var_os(ENVIRONMENT) else {
            return Ok(None);
        };
        let value = value
            .to_str()
            .context("Runyte parent context is not valid text")?;
        Self::parse(value).map(Some)
    }

    fn parse(value: &str) -> Result<Self> {
        ensure!(
            value != "standalone",
            "this integrated terminal belongs to a standalone editor; launch a persistent session outside it first"
        );
        ensure!(
            value.len() <= 65536,
            "Runyte parent context exceeds its limit"
        );
        let context: Self = serde_json::from_str(value)
            .context("invalid Runyte parent context; open a fresh integrated terminal")?;
        ensure!(
            context.protocol == crate::protocol::VERSION,
            "Runyte parent context uses an incompatible protocol; restart the persistent session"
        );
        ensure!(
            context.terminal > 0
                && context.capability.len() == 64
                && !context.metadata.is_empty()
                && context.metadata.len() <= crate::protocol::MAX_PATH_BYTES,
            "invalid Runyte parent context; open a fresh integrated terminal"
        );
        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_context_is_bound_to_host_and_terminal_and_never_debugs_secret() {
        let launch = ParentLaunch::new(Path::new("/tmp/runyte-parent-test/endpoint.json")).unwrap();
        let id = TerminalId::from_raw(1);
        let context = ParentContext::parse(&launch.context(id)).unwrap();
        assert!(launch.validates(id, &context.capability));
        assert!(!launch.validates(TerminalId::from_raw(2), &context.capability));
        let replaced =
            ParentLaunch::new(Path::new("/tmp/runyte-parent-test/endpoint.json")).unwrap();
        assert!(!replaced.validates(id, &context.capability));
        assert!(!format!("{launch:?}").contains(&launch.secret));
        assert!(
            ParentContext::parse("standalone")
                .unwrap_err()
                .to_string()
                .contains("standalone")
        );
        assert!(ParentContext::parse("not-json").is_err());
        assert!(ParentContext::parse(&"x".repeat(65537)).is_err());
        let mut stale = context.clone();
        stale.protocol = 0;
        assert!(ParentContext::parse(&serde_json::to_string(&stale).unwrap()).is_err());
        stale = context;
        stale.terminal = 0;
        assert!(ParentContext::parse(&serde_json::to_string(&stale).unwrap()).is_err());
    }
    #[test]
    fn copied_parent_capability_cannot_authorize_another_unix_session() {
        let launch = ParentLaunch::new(Path::new("/tmp/runyte-parent-test/endpoint.json")).unwrap();
        let mut terminals = crate::terminal::TerminalSessions::new();
        terminals.set_parent_launch(launch.clone());
        let id = terminals
            .open(
                crate::terminal::TerminalRequest {
                    program: "/bin/cat".into(),
                    arguments: vec![],
                    directory: std::env::temp_dir(),
                    label: "parent".to_owned(),
                },
                80,
                24,
            )
            .unwrap();
        let context = ParentContext::parse(&launch.context(id)).unwrap();
        let process = terminals.get(id).unwrap().process_id();
        assert!(terminals.validates_parent(id, &context.capability, process));
        assert!(!terminals.validates_parent(id, &context.capability, Some(std::process::id())));
        assert!(!terminals.validates_parent(id, &context.capability, None));
        terminals.close(id);
        assert!(!terminals.validates_parent(id, &context.capability, process));
    }
}
