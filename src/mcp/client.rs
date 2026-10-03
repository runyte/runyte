// SPDX-License-Identifier: MPL-2.0

use super::{Failure, Result, parse, read_frame, write_frame};
use crate::workspace::context::{
    discovery,
    storage::{self, Registration, Storage},
    wire::{self, Scope},
};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, BufReader},
    time::{Instant, timeout},
};

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}
struct Connection {
    socket: BufReader<Box<dyn Stream>>,
    scopes: BTreeSet<Scope>,
    generation: u64,
    count: usize,
    limit: usize,
    identity: String,
}
impl Connection {
    async fn refresh(mut self, identity: &storage::Identity) -> Option<Self> {
        if self.identity != identity.fingerprint() || self.count == wire::MAX_DEDUPLICATED_REQUESTS
        {
            return None;
        }
        // A metadata-only request checks the live grant and connection. Grant
        // changes close readers at the host, so stale negotiations cannot pass.
        let method = if self.scopes.contains(&Scope::EditorContextRead) {
            "buffer.list"
        } else if self.scopes.contains(&Scope::TerminalRead) {
            "terminal.list"
        } else {
            return None;
        };
        self.count += 1;
        let id = format!("r:{}", self.count);
        let response = self
            .exchange(
                &json!({"type":"request","id":id,"method":method,"params":{"offset":0,"limit":1}}),
            )
            .await
            .ok()?;
        (response["type"] == "response"
            && response["id"] == id
            && response["result"].is_object()
            && response.get("error").is_none())
        .then_some(self)
    }
    async fn exchange(&mut self, message: &Value) -> Result<Value> {
        write_frame(self.socket.get_mut(), message, self.limit).await?;
        let raw = read_frame(&mut self.socket, self.limit).await?;
        parse(&raw)
    }
    fn limits(&mut self, value: &Value) -> Result<()> {
        let limit = value["limits"]["line_bytes"]
            .as_u64()
            .filter(|v| (1024..=wire::MAX_FRAME_BYTES as u64).contains(v))
            .ok_or_else(|| Failure::new("unsupported", "Invalid context frame limit"))?;
        self.limit = self.limit.min(limit as usize);
        Ok(())
    }
    async fn connect(
        record: &Registration,
        root: &std::path::Path,
        identity: &storage::Identity,
    ) -> Result<Self> {
        // The store validates the directory and credential. Only a verified
        // same-owner endpoint in that store may receive authentication.
        #[cfg(unix)]
        if record.endpoint.parent().and_then(|p| p.canonicalize().ok())
            != Some(root.canonicalize()?)
        {
            return Err(Failure::new(
                "unavailable",
                "Endpoint is outside private context storage",
            ));
        }
        #[cfg(windows)]
        let _ = root;
        let socket = discovery::connect(record).await?;
        let mut connection = Self {
            socket: BufReader::new(Box::new(socket)),
            scopes: BTreeSet::new(),
            generation: 0,
            count: 0,
            limit: wire::MAX_FRAME_BYTES,
            identity: identity.fingerprint(),
        };
        let hello = connection
            .exchange(&json!({"type":"authenticate","credential":identity.credential()}))
            .await?;
        if hello["type"] == "registration_error" {
            return Err(Failure::new(
                "capability_denied",
                "Identity has no grant for this workspace",
            ));
        }
        let workspace: Registration = serde_json::from_value(hello["workspace"].clone())
            .map_err(|_| Failure::new("stale", "Invalid context host identity"))?;
        if workspace != *record
            || hello["type"] != "hello"
            || hello["version"] != "runyte-1"
            || !hello["features"]
                .as_array()
                .is_some_and(|v| v.contains(&json!(wire::FEATURE)))
        {
            return Err(Failure::new(
                "stale",
                "Context host identity or protocol changed",
            ));
        }
        use crate::plugin::compatibility::{ReleaseRange, Version};
        let supported = ReleaseRange::parse(">=0.3.0, <0.4.0").unwrap();
        if !hello["host_version"]
            .as_str()
            .and_then(|v| Version::parse(v).ok())
            .is_some_and(|v| supported.contains(&v))
        {
            return Err(Failure::new(
                "unsupported",
                "Host release is outside the supported range",
            ));
        }
        connection.limits(&hello)?;
        let reply = connection.exchange(&json!({"type":"register","version":"runyte-1","runyte":">=0.3.0, <0.4.0",
            "name":format!("Runyte MCP ({})",identity.name()),"commands":[],"required_features":[wire::FEATURE],"optional_features":[],
            "required_capabilities":[],"optional_capabilities":wire::CAPABILITIES})).await?;
        if reply["type"] == "registration_error" {
            return Err(Failure::new(
                "capability_denied",
                "Context registration denied",
            ));
        }
        if reply["type"] != "registered"
            || reply["features"] != json!([wire::FEATURE])
            || reply["commands"] != json!([])
            || reply["runyte"] != ">=0.3.0, <0.4.0"
        {
            return Err(Failure::new(
                "unsupported",
                "Context profile negotiation failed",
            ));
        }
        let scopes: Vec<Scope> = serde_json::from_value(reply["capabilities"].clone())
            .map_err(|_| Failure::new("unsupported", "Invalid context scopes"))?;
        connection.scopes = scopes.iter().copied().collect();
        if connection.scopes.len() != scopes.len()
            || wire::validate_scopes(&connection.scopes).is_err()
        {
            return Err(Failure::new("unsupported", "Invalid context scopes"));
        }
        connection.limits(&reply)?;
        Ok(connection)
    }
}

#[derive(Clone)]
struct Resource {
    workspace: String,
    generation: u64,
    kind: String,
    native: String,
    metadata: Value,
}

pub(super) struct Bridge {
    root: Option<PathBuf>,
    pub(super) identity: String,
    timeout: Duration,
    prefix: String,
    next: u64,
    records: BTreeMap<String, Registration>,
    connections: BTreeMap<String, Connection>,
    order: VecDeque<String>,
    resources: BTreeMap<String, Resource>,
    pub(super) deadline: Option<Instant>,
}
impl Bridge {
    pub(super) fn new(
        root: Option<PathBuf>,
        identity: String,
        timeout: Duration,
    ) -> std::io::Result<Self> {
        Ok(Self {
            root,
            identity,
            timeout,
            prefix: storage::random_token()?[..12].into(),
            next: 0,
            records: BTreeMap::new(),
            connections: BTreeMap::new(),
            order: VecDeque::new(),
            resources: BTreeMap::new(),
            deadline: None,
        })
    }
    fn token(&mut self, kind: &str) -> String {
        self.next += 1;
        format!("{kind}:{}:{}", self.prefix, self.next)
    }
    fn forget(&mut self, workspace: &str) {
        self.connections.remove(workspace);
        self.order.retain(|w| w != workspace);
        self.resources.retain(|_, r| r.workspace != workspace);
    }
    fn budget(&self) -> Duration {
        self.deadline.map_or(self.timeout, |d| {
            self.timeout
                .min(d.saturating_duration_since(Instant::now()))
        })
    }

    /// Explicit rediscovery also refreshes grants; this is never a mutation retry.
    /// At most four selected hosts are admitted, in parallel, with no scope probe
    /// followed by a second connection as in the original Python bridge.
    pub(super) async fn discover(
        &mut self,
        selector: &str,
        hidden: bool,
        offset: usize,
    ) -> Result<Value> {
        let inventory = discovery::discover(
            self.root.clone(),
            &storage::environment_fingerprint(),
            hidden,
        )
        .await?;
        let mut candidates = Vec::new();
        let mut live = BTreeSet::new();
        for endpoint in inventory.workspaces {
            let record = endpoint.registration;
            let existing = self
                .records
                .iter()
                .find(|(_, r)| **r == record)
                .map(|(k, _)| k.clone());
            let key = existing.unwrap_or_else(|| self.token("w"));
            live.insert(key.clone());
            self.records.insert(key.clone(), record.clone());
            if selector.is_empty()
                || selector == key
                || record
                    .root
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&selector.to_lowercase())
            {
                candidates.push((key, record, endpoint.label));
            }
        }
        for key in self
            .records
            .keys()
            .filter(|k| !live.contains(*k))
            .cloned()
            .collect::<Vec<_>>()
        {
            self.forget(&key);
            self.records.remove(&key);
        }
        let total = candidates.len();
        let page = candidates
            .into_iter()
            .skip(offset)
            .take(4)
            .map(|(key, record, label)| {
                let connection = self.connections.remove(&key);
                self.order.retain(|w| w != &key);
                (key, record, label, connection)
            })
            .collect::<Vec<_>>();
        while self.connections.len() + page.len() > 8 {
            if let Some(oldest) = self.order.front().cloned() {
                self.forget(&oldest);
            } else {
                break;
            }
        }
        let loaded = self
            .root
            .as_ref()
            .and_then(|root| Storage::open_existing(root.clone()).ok())
            .and_then(|store| store.load_identity(&self.identity).ok().flatten());
        let duration = self.timeout;
        let root = self.root.clone();
        let connected = stream::iter(page.into_iter().map(|(key, record, label, connection)| {
            let identity = loaded.clone();
            let root = root.clone();
            async move {
                let result = match (root, identity) {
                    (Some(root), Some(identity)) => timeout(duration, async {
                        if let Some(connection) = connection
                            && let Some(connection) = connection.refresh(&identity).await
                        {
                            return Ok(connection);
                        }
                        Connection::connect(&record, &root, &identity).await
                    })
                    .await
                    .unwrap_or_else(|_| {
                        Err(Failure::new("timeout", "Context host did not respond"))
                    }),
                    _ => Err(Failure::new(
                        "capability_denied",
                        "No paired identity; grant access in Runyte",
                    )),
                };
                (key, record, label, result)
            }
        }))
        .buffered(4)
        .collect::<Vec<_>>()
        .await;
        let mut rows = Vec::new();
        for (key, record, label, result) in connected {
            let (scopes, error) = match result {
                Ok(mut connection) => {
                    if connection.generation == 0 {
                        self.resources.retain(|_, r| r.workspace != key);
                        self.next += 1;
                        connection.generation = self.next;
                    }
                    let scopes = connection.scopes.clone();
                    self.connections.insert(key.clone(), connection);
                    self.order.push_back(key.clone());
                    (scopes, None)
                }
                Err(e) => {
                    self.forget(&key);
                    (BTreeSet::new(), Some(e.code))
                }
            };
            rows.push(json!({"workspace":key,"root":record.root,"label":label,"scopes":scopes,"unavailable_reason":error}));
        }
        Ok(
            json!({"identity":self.identity,"workspaces":rows,"next_workspace_offset":(offset+4<total).then_some(offset+4),
            "truncated":inventory.truncated,"permission_help":format!("In the target Runyte workspace, use :mcp {}. Apply permissions, then rerun find_resources; no agent restart needed.",self.identity)}),
        )
    }

    pub(super) fn scopes(&self, workspace: &str) -> BTreeSet<Scope> {
        self.connections
            .get(workspace)
            .map(|c| c.scopes.clone())
            .unwrap_or_default()
    }
    pub(super) fn resource(&self, handle: &str, kind: &str) -> Result<(String, Value)> {
        let resource = self
            .resources
            .get(handle)
            .filter(|r| {
                r.kind == kind
                    && self
                        .connections
                        .get(&r.workspace)
                        .is_some_and(|c| c.generation == r.generation)
            })
            .ok_or_else(|| {
                Failure::new(
                    "stale",
                    "Unknown or expired handle; run find_resources again",
                )
            })?;
        Ok((resource.workspace.clone(), resource.metadata.clone()))
    }
    fn wrap(&mut self, workspace: &str, generation: u64, value: &mut Value) -> Result<()> {
        if let Some(object) = value.as_object_mut() {
            // Retain only routing/revision metadata, never buffer or terminal text.
            let metadata: BTreeMap<String, Value> = object
                .iter()
                .filter(|(k, _)| ["revision", "chars", "name", "read_only"].contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            for (kind, child) in object {
                if ["buffer", "terminal", "pane", "snapshot", "proposal"].contains(&kind.as_str())
                    && child.is_string()
                {
                    let native = child.as_str().unwrap();
                    let found = self
                        .resources
                        .iter()
                        .find(|(_, r)| {
                            r.workspace == workspace
                                && r.generation == generation
                                && r.kind == *kind
                                && r.native == native
                        })
                        .map(|(k, _)| k.clone());
                    let handle = if let Some(handle) = found {
                        handle
                    } else {
                        if self.resources.len() == 4096 {
                            return Err(Failure::new(
                                "limit_exceeded",
                                "Resource handle limit reached; rediscover workspace",
                            ));
                        }
                        let handle = self.token(&kind[..1]);
                        self.resources.insert(
                            handle.clone(),
                            Resource {
                                workspace: workspace.into(),
                                generation,
                                kind: kind.clone(),
                                native: native.into(),
                                metadata: Value::Null,
                            },
                        );
                        handle
                    };
                    let resource = self.resources.get_mut(&handle).unwrap();
                    if !resource.metadata.is_object() {
                        resource.metadata = json!({});
                    }
                    for (k, v) in &metadata {
                        resource.metadata[k] = v.clone();
                    }
                    *child = json!(handle);
                } else {
                    self.wrap(workspace, generation, child)?;
                }
            }
        } else if let Some(array) = value.as_array_mut() {
            for child in array {
                self.wrap(workspace, generation, child)?;
            }
        }
        Ok(())
    }
    pub(super) async fn request(
        &mut self,
        workspace: &str,
        method: &str,
        mut params: Value,
    ) -> Result<Value> {
        let duration = self.budget();
        let connection = self.connections.get_mut(workspace).ok_or_else(|| {
            Failure::new("stale", "Run find_resources to discover this workspace")
        })?;
        for kind in ["buffer", "terminal", "pane", "snapshot", "proposal"] {
            if let Some(handle) = params.get(kind).and_then(Value::as_str) {
                let resource = self
                    .resources
                    .get(handle)
                    .filter(|r| {
                        r.workspace == workspace
                            && r.generation == connection.generation
                            && r.kind == kind
                    })
                    .ok_or_else(|| {
                        Failure::new("stale", "Handle does not belong to this connection")
                    })?;
                params[kind] = json!(resource.native);
            }
        }
        let message = json!({"type":"request","id":format!("r:{}",connection.count+1),"method":method,"params":params});
        // Reuse the host's strict typed validation, including text controls and
        // byte limits, before sending anything. No generic method is exposed.
        let parsed = wire::parse_request(&message.to_string())
            .map_err(|e| Failure::new("invalid_argument", e.message))?;
        if !connection.scopes.contains(&parsed.request.required_scope()) {
            return Err(Failure::new(
                "capability_denied",
                format!(
                    "Missing {}. In Runyte use :mcp {}, apply permissions, then find_resources again.",
                    parsed.request.required_scope().capability(),
                    self.identity
                ),
            ));
        }
        if connection.count == wire::MAX_DEDUPLICATED_REQUESTS {
            self.forget(workspace);
            return Err(Failure::new(
                "stale",
                "Request budget exhausted before sending; run find_resources again",
            ));
        }
        if duration.is_zero() {
            return Err(Failure::new("timeout", "Search time limit reached"));
        }
        connection.count += 1;
        let generation = connection.generation;
        let result = timeout(duration, connection.exchange(&message)).await;
        let response = match result {
            Ok(Ok(value))
                if value["type"] == "response"
                    && value["id"] == message["id"]
                    && (value.get("result").is_some() != value.get("error").is_some())
                    && (value["result"].is_object()
                        || (value["error"]["code"].is_string()
                            && value["error"]["message"].is_string())) =>
            {
                value
            }
            _ => {
                self.forget(workspace);
                return Err(
                    if [
                        "buffer.edit",
                        "buffer.append",
                        "terminal.input.propose",
                        "terminal.input.cancel",
                    ]
                    .contains(&method)
                    {
                        Failure::new(
                            "outcome_unknown",
                            "Delivery is uncertain; do not retry automatically",
                        )
                    } else {
                        Failure::new(
                            "unavailable",
                            "Connection closed or timed out; run find_resources again",
                        )
                    },
                );
            }
        };
        if let Some(error) = response.get("error") {
            if error["code"] == "capability_denied"
                || (error["code"] == "limit_exceeded"
                    && matches!(method, "buffer.list" | "terminal.list"))
            {
                // Inventory handles live for the reader's lifetime. Refreshing
                // an already-known resource cannot renew an exhausted reader.
                self.forget(workspace);
            }
            return Err(Failure::new(
                error["code"].as_str().unwrap(),
                error["message"].as_str().unwrap(),
            ));
        }
        let mut data = response["result"].clone();
        if let Some(object) = data.as_object_mut() {
            object.remove("workspace");
        }
        if method == "buffer.append" {
            data["chars"] = data["to"].clone();
        }
        if method == "buffer.edit" {
            for resource in self.resources.values_mut().filter(|r| {
                r.workspace == workspace && r.kind == "buffer" && r.native == params["buffer"]
            }) {
                if let Some(object) = resource.metadata.as_object_mut() {
                    object.remove("chars");
                }
            }
        }
        self.wrap(workspace, generation, &mut data)?;
        self.order.retain(|w| w != workspace);
        self.order.push_back(workspace.into());
        Ok(json!({"workspace":workspace,"source_content":"untrusted","data":data}))
    }
}
