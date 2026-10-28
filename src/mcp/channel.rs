// SPDX-License-Identifier: MPL-2.0

//! The agent channel tools. The adapter reports where the agent works and the
//! terminal it runs in, keeps its place in each workspace's channel, and joins
//! it again after a reconnect, so an agent names its provider and task once.

use super::{
    Failure, Result,
    client::{Bridge, Joined},
};
use crate::workspace::context::channel::Location;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use tokio::time::Instant;

pub(super) const TOOLS: [&str; 4] = [
    "join_channel",
    "post_message",
    "read_messages",
    "list_participants",
];

/// How long a computed location is reused for the same directory.
const LOCATION_REUSE: Duration = Duration::from_secs(5);
/// Longest pause between polls while a read waits for something new.
const LONGEST_POLL: Duration = Duration::from_secs(2);

/// Missing or unusable launch context never becomes a terminal claim. The
/// native binding action remains available to daemon-launched MCP clients.
pub(super) fn terminal_claim(value: Option<&str>) -> Option<Value> {
    let parent = crate::workspace::parent::ParentContext::parse(value?).ok()?;
    Some(json!({"terminal":parent.terminal,"capability":parent.capability}))
}

impl Bridge {
    pub(super) async fn channel_call(&mut self, name: &str, args: Value) -> Result<Value> {
        let workspace = self
            .channel_workspace(args["workspace"].as_str().unwrap_or(""))
            .await?;
        // A directory the agent names stays its place until it names another,
        // so leaving it out later does not read as moving back.
        let directory = args["directory"]
            .as_str()
            .map(PathBuf::from)
            .filter(|directory| directory.is_absolute());
        if let Some(directory) = &directory
            && let Some(joined) = self.joined.get_mut(&workspace)
        {
            joined.directory = Some(directory.clone());
        }
        let directory = directory.or_else(|| {
            self.joined
                .get(&workspace)
                .and_then(|j| j.directory.clone())
        });
        match name {
            "join_channel" => {
                let location = self.location(directory).await;
                let provider = args["provider"].as_str().unwrap_or_default().to_owned();
                let task = args["task"].as_str().unwrap_or_default().to_owned();
                self.channel_join(&workspace, provider, task, location)
                    .await
            }
            "post_message" => {
                let location = self.location(directory).await;
                self.channel_rejoin(&workspace, &location).await?;
                let mut params = json!({"text":args["text"],"location":location});
                if let Some(to) = args["to"].as_str() {
                    params["to"] = json!(to);
                }
                if let Some(nudge) = args.get("nudge") {
                    if !self.nudge_supported(&workspace) {
                        return Err(Failure::new(
                            "unsupported",
                            "This Runyte cannot nudge; post without nudge, or update Runyte",
                        ));
                    }
                    let transport = args["nudge_transport"].as_str().unwrap_or("notification");
                    if transport != "terminal" && !self.notify_supported(&workspace) {
                        return Err(Failure::new(
                            "unsupported",
                            "Host does not support channel notifications; explicit nudge_transport=terminal selects the legacy backup",
                        ));
                    }
                    params["nudge"] = nudge.clone();
                    if self.notify_supported(&workspace) {
                        params["nudge_transport"] = json!(transport);
                    }
                }
                self.request(&workspace, "channel.post", params).await
            }
            "read_messages" => {
                let location = self.location(directory).await;
                self.channel_rejoin(&workspace, &location).await?;
                let mut since = args["since"]
                    .as_u64()
                    .unwrap_or_else(|| self.joined.get(&workspace).map_or(0, |j| j.next));
                let limit = args["limit"].as_u64().unwrap_or(50);
                let until = Instant::now()
                    + Duration::from_secs(args["wait_seconds"].as_u64().unwrap_or(0));
                let mut pause = Duration::from_millis(250);
                let mut last = None;
                loop {
                    let reply = match self
                        .request(
                            &workspace,
                            "channel.read",
                            json!({"since":since,"limit":limit}),
                        )
                        .await
                    {
                        Ok(reply) => reply,
                        // A wait that already had an answer ends with it
                        // rather than with the interruption; the next call
                        // reconnects and joins again.
                        Err(error) => return last.ok_or(error),
                    };
                    since = reply["data"]["next"].as_u64().unwrap_or(since);
                    if let Some(joined) = self.joined.get_mut(&workspace) {
                        joined.next = since;
                    }
                    let empty = reply["data"]["messages"]
                        .as_array()
                        .is_none_or(Vec::is_empty);
                    if !empty || Instant::now() >= until {
                        return Ok(reply);
                    }
                    last = Some(reply);
                    // Backing off keeps a long wait to a few requests.
                    tokio::time::sleep(pause.min(until - Instant::now())).await;
                    pause = (pause * 2).min(LONGEST_POLL);
                }
            }
            _ => self.request(&workspace, "channel.who", json!({})).await,
        }
    }

    /// The workspace to use: the one named, or else the only one whose grant
    /// includes the channel, preferring one this agent already joined.
    pub(super) async fn channel_workspace(&mut self, selector: &str) -> Result<String> {
        let inventory = self.discover(selector, false, 0).await?;
        let candidates = inventory["workspaces"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|w| {
                w["scopes"]
                    .as_array()
                    .is_some_and(|s| s.contains(&json!("agent_channel")))
            })
            .filter_map(|w| w["workspace"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        if let Some(joined) = candidates.iter().find(|w| self.joined.contains_key(*w))
            && (selector.is_empty() || candidates.len() == 1)
        {
            return Ok(joined.clone());
        }
        let discovered = inventory["workspaces"]
            .as_array()
            .is_some_and(|w| !w.is_empty());
        match candidates.as_slice() {
            [only] => Ok(only.clone()),
            // Nothing to grant: blaming permissions here sends the person to
            // a dialog that is already correct.
            [] if !discovered && inventory["hidden_workspaces"].is_u64() => {
                Err(Failure::new("unavailable", super::client::HIDDEN_HELP))
            }
            [] if !discovered && selector.is_empty() => Err(Failure::new(
                "unavailable",
                "No running Runyte workspace was discovered, so there is no agent channel to join.",
            )),
            [] if !discovered => Err(Failure::new(
                "unavailable",
                format!("No running Runyte workspace matches {selector}."),
            )),
            [] => Err(Failure::new(
                "capability_denied",
                format!(
                    "No workspace grants {} the agent channel. In Runyte use :mcp {} and turn on 6; a Runyte without the agent channel has no 6 to turn on.",
                    self.identity, self.identity
                ),
            )),
            _ => Err(Failure::new(
                "invalid_argument",
                "Several workspaces carry an agent channel; pass workspace",
            )),
        }
    }

    async fn channel_join(
        &mut self,
        workspace: &str,
        provider: String,
        task: String,
        location: Location,
    ) -> Result<Value> {
        let mut params = json!({"provider":provider,"task":task,"location":location});
        if self.notify_supported(workspace) {
            params["notifications"] = json!(self.wake_mode != super::wake::WakeMode::Disabled);
        }
        if let Some(client) = &self.client {
            params["client"] = json!(client);
        }
        // An integrated terminal of a persistent session hands its children
        // a capability naming it; the host checks it before believing it.
        let parent = std::env::var(crate::workspace::parent::ENVIRONMENT).ok();
        if let Some(terminal) = terminal_claim(parent.as_deref()) {
            params["terminal"] = terminal;
        }
        let earlier = self.joined.get(workspace).map(|j| j.next);
        let directory = PathBuf::from(&location.directory);
        if let Some(joined) = self.joined.get(workspace) {
            params["resume"] = json!(joined.resume);
        }
        let mut reply = self.request(workspace, "channel.join", params).await?;
        // The token proves this process to the host; nothing else needs it.
        let resume = reply["data"]
            .as_object_mut()
            .and_then(|data| data.remove("resume"))
            .and_then(|resume| resume.as_str().map(str::to_owned))
            .ok_or_else(|| Failure::new("unavailable", "Invalid channel join"))?;
        let generation = self.generation(workspace).unwrap_or_default();
        self.joined.insert(
            workspace.to_owned(),
            Joined {
                resume,
                provider,
                task,
                directory: directory.is_absolute().then_some(directory),
                generation,
                // A first join reads the conversation so far.
                next: earlier.unwrap_or(0),
            },
        );
        Ok(reply)
    }

    /// Joins again, as the same participant, when the connection the join
    /// was made on has been renewed.
    async fn channel_rejoin(&mut self, workspace: &str, location: &Location) -> Result<()> {
        let Some(joined) = self.joined.get(workspace) else {
            return Err(Failure::new(
                "capability_denied",
                "Call join_channel first, naming your provider and main task",
            ));
        };
        if self.generation(workspace) == Some(joined.generation) {
            return Ok(());
        }
        let (provider, task) = (joined.provider.clone(), joined.task.clone());
        self.channel_join(workspace, provider, task, location.clone())
            .await
            .map(|_| ())
    }

    /// Where this agent works: the directory it names, or else the one this
    /// server was started in, and the repository, worktree and branch around
    /// it. Reused briefly, so a burst of posts runs Git once.
    async fn location(&mut self, directory: Option<PathBuf>) -> Location {
        let directory = directory.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        if let Some((cached, at, location)) = &self.location
            && *cached == directory
            && at.elapsed() < LOCATION_REUSE
        {
            return location.clone();
        }
        let probed = directory.clone();
        let location = tokio::task::spawn_blocking(move || locate(&probed))
            .await
            .unwrap_or_default();
        self.location = Some((directory, Instant::now(), location.clone()));
        location
    }
}

fn locate(directory: &std::path::Path) -> Location {
    use crate::git::GitProvider;
    let mut location = Location {
        directory: clean(&directory.display().to_string()).unwrap_or_else(|| "unknown".into()),
        ..Location::default()
    };
    let Some(git) = crate::git::GitCliProvider::from_environment() else {
        return location;
    };
    let Ok(Some(repository)) = git.discover(directory) else {
        return location;
    };
    location.worktree = clean(&repository.workdir().display().to_string());
    let common = repository.common_dir();
    let main = if common.file_name().is_some_and(|name| name == ".git") {
        common.parent().unwrap_or(common)
    } else {
        common
    };
    location.repository = clean(&main.display().to_string());
    location.branch = git.branches(&repository).ok().and_then(|branches| {
        branches
            .into_iter()
            .find(|branch| branch.current)
            .and_then(|branch| clean(&branch.name))
    });
    location
}

/// A location field the host will accept, or none: an unusual path must not
/// stop an agent from joining.
fn clean(value: &str) -> Option<String> {
    (!value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
}
