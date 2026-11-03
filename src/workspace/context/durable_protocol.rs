// SPDX-License-Identifier: MPL-2.0

//! Opt-in durable channel requests. Credentials are adapter-owned, never tool arguments.
use super::channel::Location;
use crate::plugin::application::{Error, ErrorCode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Credential(pub String);
impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credential([redacted])")
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub credential: Credential,
    pub store: String,
    pub operation: Operation,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Register {
        provider: String,
        task: String,
    },
    Resume {},
    Create {
        name: String,
        fixed: bool,
        members: Vec<String>,
    },
    Subscribe {
        channel: String,
    },
    Post {
        channel: String,
        nonce: String,
        issued_ms: u64,
        text: String,
        location: Location,
    },
    Read {
        channel: String,
        limit: usize,
    },
    History {
        channel: String,
        limit: usize,
        before: Option<u64>,
    },
    Acknowledge {
        channel: String,
        receipt: String,
    },
}
impl Command {
    pub fn validate(&self) -> Result<(), Error> {
        let bounded =
            |s: &str, n| !s.is_empty() && s.len() <= n && !s.chars().any(char::is_control);
        let hex = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
        let valid = hex(&self.credential.0)
            && hex(&self.store)
            && match &self.operation {
                Operation::Register { provider, task } => {
                    bounded(provider, 32) && bounded(task, 800)
                }
                Operation::Resume {} => true,
                Operation::Create {
                    name,
                    fixed,
                    members,
                } => {
                    bounded(name, 128)
                        && members.len() < 64
                        && (*fixed || members.is_empty())
                        && members.iter().all(|v| bounded(v, 64))
                }
                Operation::Subscribe { channel } => bounded(channel, 64),
                Operation::Post {
                    channel,
                    nonce,
                    issued_ms,
                    text,
                    location,
                } => {
                    bounded(channel, 64)
                        && hex(nonce)
                        && *issued_ms <= i64::MAX as u64
                        && !text.trim().is_empty()
                        && text.len() <= super::channel::MAX_TEXT_BYTES
                        && !text
                            .chars()
                            .any(|c| c.is_control() && c != '\n' && c != '\t')
                        && location.validate().is_ok()
                }
                Operation::Read { channel, limit } => {
                    bounded(channel, 64) && (1..=200).contains(limit)
                }
                Operation::History {
                    channel,
                    limit,
                    before,
                } => {
                    bounded(channel, 64)
                        && (1..=200).contains(limit)
                        && before.is_none_or(|v| v <= i64::MAX as u64)
                }
                Operation::Acknowledge { channel, receipt } => bounded(channel, 64) && hex(receipt),
            };
        if valid {
            Ok(())
        } else {
            Err(Error::new(
                ErrorCode::InvalidArgument,
                "Invalid durable channel operation",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    #[test]
    fn wire_actions_are_strict_bounded_and_credentials_redacted() {
        let secret = "a".repeat(64);
        let parse = |op| {
            super::super::wire::parse_request(&json!({"type":"request","id":"one","method":"channel.durable","params":{"credential":secret,"store":"b".repeat(64),"operation":op}}).to_string())
        };
        for op in [
            json!({"action":"resume","extra":1}),
            json!({"action":"unknown"}),
            json!({"action":"read","channel":"c","limit":0}),
            json!({"action":"read","channel":"c","limit":201}),
            json!({"action":"acknowledge","channel":"c","receipt":"bad"}),
            json!({"action":"register","provider":"x".repeat(33),"task":"t"}),
        ] {
            assert!(parse(op).is_err());
        }
        for op in [
            json!({"action":"resume"}),
            json!({"action":"register","provider":"test","task":"t"}),
            json!({"action":"create","name":"n","fixed":false,"members":[]}),
            json!({"action":"subscribe","channel":"c"}),
            json!({"action":"read","channel":"c","limit":20}),
            json!({"action":"history","channel":"c","limit":20,"before":null}),
            json!({"action":"acknowledge","channel":"c","receipt":"c".repeat(64)}),
            json!({"action":"post","channel":"c","nonce":"c".repeat(64),"issued_ms":1,"text":"hi","location":{"directory":"/fixture","repository":null,"worktree":null,"branch":null}}),
        ] {
            let parsed = parse(op).unwrap();
            assert_eq!(
                parsed.request.required_scope(),
                super::super::wire::Scope::AgentChannel
            );
            assert!(!format!("{parsed:?}").contains(&secret));
        }
    }
}
