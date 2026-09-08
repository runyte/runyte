// SPDX-License-Identifier: MPL-2.0

//! Epoch 2 wire values. No editor or frontend types cross this boundary.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const VERSION: &str = "runyte-experimental-2";
pub const MAX_COMMANDS: usize = 64;
pub const MAX_REQUESTS: usize = 16;
pub const MAX_JOBS: usize = 4;
pub const MAX_QUEUE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
pub enum Api {
    #[default]
    #[serde(rename = "runyte-experimental-1")]
    Epoch1,
    #[serde(rename = "runyte-experimental-2")]
    Epoch2,
}
impl Api {
    pub fn version(self) -> &'static str {
        match self {
            Self::Epoch1 => super::VERSION,
            Self::Epoch2 => VERSION,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidArgument,
    Unsupported,
    CapabilityDenied,
    NotFound,
    Closed,
    Stale,
    Conflict,
    ReadOnly,
    Busy,
    LimitExceeded,
    Cancelled,
    Timeout,
    Unavailable,
    Internal,
    OutcomeUnknown,
    NoFrontend,
    ContextChanged,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}
impl Error {
    pub fn new(code: ErrorCode, message: &str) -> Self {
        Self {
            code,
            message: message.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    #[serde(default)]
    pub arguments: Vec<super::arguments::Argument>,
    #[serde(default)]
    pub primary: bool,
    pub name: String,
    pub description: String,
    pub context: CommandContext,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandContext {
    Workspace,
    Buffer,
    View,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Register {
        version: String,
        name: String,
        commands: Vec<Registration>,
        required_capabilities: BTreeSet<String>,
        optional_capabilities: BTreeSet<String>,
    },
    Request {
        id: String,
        #[serde(flatten)]
        request: Request,
    },
    Response {
        id: String,
        #[serde(flatten)]
        outcome: CommandResponse,
    },
}

// The untagged response variants enforce exactly one terminal outcome.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommandResponse {
    Validation {
        result: super::interaction::ValidationResult,
    },
    Resource {
        result: super::provider::Response,
    },
    Success {
        result: CommandResult,
    },
    Failure {
        error: Error,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandResult {
    pub job: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum Request {
    #[serde(rename = "process.start")]
    ProcessStart(super::process::Start),
    #[serde(rename = "terminal.open")]
    TerminalOpen(super::handoff::TerminalOpen),
    #[serde(rename = "external.open")]
    ExternalOpen {
        invocation: String,
        target: super::handoff::Target,
    },
    #[serde(rename = "notification.publish")]
    NotificationPublish(super::handoff::Notification),
    #[serde(rename = "process.get")]
    ProcessGet { process: String },
    #[serde(rename = "process.read")]
    ProcessRead {
        process: String,
        stream: super::process::Stream,
        offset: u64,
        limit: usize,
    },
    #[serde(rename = "process.write")]
    ProcessWrite {
        process: String,
        data: String,
        #[serde(default)]
        eof: bool,
    },
    #[serde(rename = "process.close")]
    ProcessClose { process: String },
    #[serde(rename = "event.subscribe")]
    EventSubscribe {
        sources: Vec<super::observation::Source>,
    },
    #[serde(rename = "event.unsubscribe")]
    EventUnsubscribe { subscription: String },
    #[serde(rename = "event.resync")]
    EventResync { subscription: String },
    #[serde(rename = "resource.inspect")]
    ResourceInspect {
        buffer: String,
        expected_revision: String,
        invocation: String,
    },
    #[serde(rename = "resource.rebind")]
    ResourceRebind {
        buffer: String,
        expected_revision: String,
    },
    #[serde(rename = "provider.register")]
    ProviderRegister(super::provider::Registration),
    #[serde(rename = "resource.open")]
    ResourceOpen {
        plugin: String,
        provider: String,
        key: String,
        invocation: Option<String>,
    },
    #[serde(rename = "buffer.save")]
    BufferSave {
        buffer: String,
        expected_revision: String,
    },
    #[serde(rename = "buffer.close")]
    BufferClose {
        buffer: String,
        expected_revision: String,
    },
    #[serde(rename = "buffer.create")]
    BufferCreate {
        path: String,
        text: String,
        invocation: Option<String>,
    },
    #[serde(rename = "ui.form")]
    UiForm {
        invocation: String,
        title: String,
        fields: Vec<super::interaction::Field>,
    },
    #[serde(rename = "ui.prompt")]
    UiPrompt {
        invocation: String,
        title: String,
        field: super::interaction::Field,
    },
    #[serde(rename = "ui.pick")]
    UiPick {
        invocation: String,
        title: String,
        choices: Vec<String>,
    },
    #[serde(rename = "ui.confirm")]
    UiConfirm {
        invocation: String,
        title: String,
        message: String,
    },
    #[serde(rename = "ui.dismiss")]
    UiDismiss { surface: String },
    #[serde(rename = "filesystem.stat")]
    FilesystemStat {
        path: String,
        expected_revision: Option<String>,
    },
    #[serde(rename = "filesystem.list")]
    FilesystemList {
        path: String,
        offset: usize,
        limit: usize,
        expected_revision: Option<String>,
    },
    #[serde(rename = "filesystem.prepare")]
    FilesystemPrepare {
        directory: String,
        expected_revision: String,
        intent: super::filesystem::Intent,
    },
    #[serde(rename = "filesystem.apply")]
    FilesystemApply { plan: String, invocation: String },
    #[serde(rename = "filesystem.cancel")]
    FilesystemCancel { plan: String },
    #[serde(rename = "filesystem.release")]
    FilesystemRelease { directory: String },
    #[serde(rename = "staging.create")]
    StagingCreate { job: String, bytes: usize },
    #[serde(rename = "staging.prepare")]
    StagingPrepare {
        staging: String,
        directory: String,
        expected_revision: String,
        destination: String,
        sha256: String,
    },
    #[serde(rename = "staging.close")]
    StagingClose { staging: String },
    #[serde(rename = "buffer.open")]
    BufferOpen {
        path: String,
        invocation: Option<String>,
    },
    #[serde(rename = "buffer.list")]
    BufferList { offset: usize, limit: usize },
    #[serde(rename = "pane.list")]
    PaneList(Empty),
    #[serde(rename = "buffer.read")]
    BufferRead {
        buffer: String,
        expected_revision: String,
        from: usize,
        to: usize,
    },
    #[serde(rename = "buffer.edit")]
    BufferEdit {
        buffer: String,
        expected_revision: String,
        changes: Vec<super::editor::Change>,
    },
    #[serde(rename = "buffer.snapshot.open")]
    SnapshotOpen {
        buffer: String,
        expected_revision: String,
    },
    #[serde(rename = "buffer.snapshot.read")]
    SnapshotRead {
        snapshot: String,
        from: usize,
        to: usize,
    },
    #[serde(rename = "buffer.snapshot.close")]
    SnapshotClose { snapshot: String },
    #[serde(rename = "selection.get")]
    SelectionGet { pane: String },
    #[serde(rename = "selection.set")]
    SelectionSet {
        pane: String,
        buffer: String,
        expected_revision: String,
        ranges: Vec<super::editor::Range>,
        primary: usize,
    },
    #[serde(rename = "view.create")]
    ViewCreate { model: super::view::Model },
    #[serde(rename = "view.get")]
    ViewGet { view: String },
    #[serde(rename = "view.query.set")]
    ViewQuerySet {
        view: String,
        expected_revision: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_query_revision: Option<String>,
        text: String,
    },
    #[serde(rename = "view.publish")]
    ViewPublish {
        view: String,
        expected_revision: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_query_revision: Option<String>,
        model: super::view::Model,
    },
    #[serde(rename = "view.patch")]
    ViewPatch {
        view: String,
        expected_revision: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_query_revision: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        header: Option<super::view::Header>,
        #[serde(deserialize_with = "super::view::decode_operations")]
        operations: Vec<super::view::Operation>,
    },
    #[serde(rename = "view.stage.open")]
    ViewStageOpen {
        view: String,
        expected_revision: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_query_revision: Option<String>,
        kind: super::view::StageKind,
        bytes: usize,
    },
    #[serde(rename = "view.stage.write")]
    ViewStageWrite {
        stage: String,
        offset: usize,
        text: String,
    },
    #[serde(rename = "view.stage.commit")]
    ViewStageCommit { stage: String },
    #[serde(rename = "view.stage.close")]
    ViewStageClose { stage: String },
    #[serde(rename = "view.snapshot.open")]
    ViewSnapshotOpen {
        view: String,
        expected_revision: String,
    },
    #[serde(rename = "view.snapshot.read")]
    ViewSnapshotRead {
        snapshot: String,
        offset: usize,
        limit: usize,
    },
    #[serde(rename = "view.snapshot.close")]
    ViewSnapshotClose { snapshot: String },
    #[serde(rename = "view.close")]
    ViewClose { view: String },
    #[serde(rename = "pane.show")]
    PaneShow { invocation: String, view: String },
    #[serde(rename = "workspace.info")]
    WorkspaceInfo(Empty),
    #[serde(rename = "job.create")]
    JobCreate {
        title: String,
        deadline_seconds: u64,
    },
    #[serde(rename = "job.get")]
    JobGet { job: String },
    #[serde(rename = "job.update")]
    JobUpdate { job: String, progress: u8 },
    #[serde(rename = "job.finish")]
    JobFinish { job: String, state: TerminalState },
    #[serde(rename = "job.cancel")]
    JobCancel { job: String },
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalState {
    Succeeded,
    Failed,
    Cancelled,
    OutcomeUnknown,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    OutcomeUnknown,
}
impl JobState {
    pub fn active(self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }
}
impl From<TerminalState> for JobState {
    fn from(state: TerminalState) -> Self {
        match state {
            TerminalState::Succeeded => Self::Succeeded,
            TerminalState::Failed => Self::Failed,
            TerminalState::Cancelled => Self::Cancelled,
            TerminalState::OutcomeUnknown => Self::OutcomeUnknown,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub job: String,
    pub title: String,
    pub state: JobState,
    pub progress: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub process_handles: usize,
    pub process_io_bytes: usize,
    pub process_output_bytes: usize,
    pub process_write_seconds: u64,
    pub line_bytes: usize,
    pub commands: usize,
    pub requests: usize,
    pub finite_jobs: usize,
    pub control_queue_messages: usize,
    pub control_queue_bytes: usize,
    pub control_deadline_seconds: u64,
    pub job_deadline_seconds: u64,
    pub cancellation_seconds: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            process_handles: super::process::MAX_HANDLES,
            process_io_bytes: super::process::MAX_IO_BYTES,
            process_output_bytes: super::process::MAX_OUTPUT_BYTES,
            process_write_seconds: 5,
            line_bytes: super::MAX_BYTES,
            commands: MAX_COMMANDS,
            requests: MAX_REQUESTS,
            finite_jobs: MAX_JOBS,
            control_queue_messages: 32,
            control_queue_bytes: MAX_QUEUE_BYTES,
            control_deadline_seconds: 10,
            job_deadline_seconds: 3600,
            cancellation_seconds: 2,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMessage {
    #[serde(rename = "request")]
    ValidationRequest {
        id: String,
        method: &'static str,
        params: super::interaction::ValidationRequest,
    },
    #[serde(rename = "request")]
    ResourceRequest {
        id: String,
        #[serde(flatten)]
        request: super::provider::Request,
    },
    #[serde(rename = "request")]
    InputRequest {
        id: String,
        method: &'static str,
        params: super::interaction::Submission,
    },
    Hello {
        version: &'static str,
        capabilities: Vec<&'static str>,
        limits: Limits,
    },
    Registered {
        commands: Vec<String>,
        capabilities: BTreeSet<String>,
        limits: Limits,
    },
    Request {
        id: String,
        method: &'static str,
        params: Invocation,
    },
    Response {
        id: String,
        #[serde(flatten)]
        outcome: Response,
    },
    Event {
        sequence: String,
        event: &'static str,
        data: EventData,
    },
}
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum EventData {
    ValidationCancelled(super::interaction::ValidationCancelled),
    Observation(super::observation::Change),
    ObservationResync(super::observation::ResyncRequired),
    ObservationAction(super::observation::ActionEvent),
    ResourceReleased { job: String },
    ResourceFinished(super::provider::Finished),
    FilesystemFinished(super::filesystem::Finished),
    FilesystemStarted { plan: String, job: String },
    Job(Job),
    ViewClosed { view: String },
}
impl From<Job> for EventData {
    fn from(job: Job) -> Self {
        Self::Job(job)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
    Success { result: ResultValue },
    Failure { error: Error },
}
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum ResultValue {
    TerminalOpened {
        terminal: String,
    },
    Process(super::process::Info),
    ProcessRead(super::process::Read),
    ProcessWrite(super::process::Write),
    ViewQuery {
        view: String,
        revision: String,
        query: super::view::Query,
    },
    ViewInfo {
        view: String,
        revision: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        query: Option<super::view::Query>,
        bytes: usize,
        rows: usize,
    },
    ViewStage {
        stage: String,
        bytes: usize,
    },
    ViewOffset {
        offset: usize,
    },
    ViewSnapshot {
        snapshot: String,
        revision: String,
        bytes: usize,
    },
    ViewChunk {
        offset: usize,
        text: String,
        eof: bool,
    },
    Observation(super::observation::Baseline),
    Staging {
        staging: String,
        path: String,
    },
    Stat(super::filesystem::Stat),
    Surface {
        surface: String,
    },
    Directory {
        directory: String,
        revision: String,
        entries: Vec<super::filesystem::Entry>,
        next: Option<usize>,
    },
    FilesystemPlan {
        plan: String,
        operations: Vec<String>,
    },
    Opened {
        buffer: String,
        revision: String,
    },
    Buffers {
        buffers: Vec<super::editor::Buffer>,
        next: Option<usize>,
    },
    Panes {
        panes: Vec<super::editor::Pane>,
    },
    Text {
        revision: String,
        from: usize,
        to: usize,
        text: String,
    },
    Edited {
        buffer: String,
        revision: String,
        changes: usize,
    },
    Snapshot {
        snapshot: String,
        revision: String,
        chars: usize,
    },
    Selection {
        pane: String,
        buffer: String,
        revision: String,
        ranges: Vec<super::editor::Range>,
        primary: usize,
    },
    Workspace {
        name: String,
    },
    View {
        view: String,
        revision: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        query: Option<super::view::Query>,
        model: std::sync::Arc<super::view::Model>,
    },
    Empty(Empty),
    Job(Job),
}
#[derive(Clone, Debug, Serialize)]
pub struct Invocation {
    pub arguments: BTreeMap<String, super::arguments::Scalar>,
    pub command: String,
    pub context: CommandContext,
    pub pane: String,
    pub selection_revision: String,
    pub buffer: Option<String>,
    pub buffer_revision: Option<String>,
    pub view: Option<String>,
    pub model_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_revision: Option<String>,
    pub rows: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct CapturedContext {
    pub foreground_allowed: bool,
    pub action: Option<u64>,
    pub pane: usize,
    pub buffer: usize,
    pub terminal: Option<crate::terminal::TerminalId>,
    pub attachment: u64,
    pub foreground: u64,
}

/// Bounded host ownership, independent of a frontend and the ten-second control timer.
pub(crate) struct Instance {
    pub notification_at: Option<std::time::Instant>,
    pub processes: BTreeSet<String>,
    pub model_requests: BTreeMap<String, super::view::Pending>,
    pub view_stages: BTreeMap<String, super::view::Stage>,
    pub view_snapshots: BTreeMap<String, super::view::OwnedSnapshot>,
    pub sensitive_input_requests: BTreeSet<String>,
    pub validation: Option<super::interaction::PendingValidation>,
    pub retired_validations: BTreeSet<String>,
    pub observations: super::observation::Registry,
    pub provider_requests: usize,
    pub providers: BTreeMap<String, super::provider::Registration>,
    pub input_surfaces: BTreeSet<String>,
    pub directories: BTreeMap<String, super::filesystem::Directory>,
    pub plans: BTreeMap<String, crate::fs_plan::FsPlan>,
    pub staging: BTreeMap<String, super::staging::Issued>,
    pub staging_plans: BTreeMap<String, String>,
    pub local_requests: BTreeMap<String, super::filesystem::Pending>,
    pub capabilities: BTreeSet<String>,
    pub requests: BTreeMap<String, CapturedContext>,
    pub views: BTreeMap<String, super::view::View>,
    pub buffers: BTreeMap<String, usize>,
    pub panes: BTreeMap<String, usize>,
    pub snapshots: BTreeMap<String, super::editor::Snapshot>,
    pub deadlines: BTreeMap<String, std::time::Instant>,
    pub retained_payload: usize,
    pub command_contexts: BTreeMap<String, CommandContext>,
    pub primary_commands: BTreeSet<String>,
    pub command_arguments: BTreeMap<String, Vec<super::arguments::Argument>>,
    pub jobs: BTreeMap<String, Job>,
    pub finished_jobs: std::collections::VecDeque<String>,
    pub job_actions: BTreeMap<String, Option<u64>>,
    pub last_request: u64,
    pub next_handle: u64,
    pub generation: String,
    pub sequence: u64,
}

impl Default for Instance {
    fn default() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            notification_at: None,
            processes: Default::default(),
            model_requests: Default::default(),
            view_stages: Default::default(),
            view_snapshots: Default::default(),
            sensitive_input_requests: Default::default(),
            validation: None,
            retired_validations: Default::default(),
            observations: Default::default(),
            provider_requests: 0,
            providers: Default::default(),
            input_surfaces: Default::default(),
            directories: Default::default(),
            plans: Default::default(),
            staging: Default::default(),
            staging_plans: Default::default(),
            local_requests: Default::default(),
            capabilities: Default::default(),
            requests: Default::default(),
            views: Default::default(),
            buffers: Default::default(),
            panes: Default::default(),
            snapshots: Default::default(),
            deadlines: Default::default(),
            retained_payload: 0,
            command_contexts: Default::default(),
            primary_commands: Default::default(),
            command_arguments: Default::default(),
            jobs: Default::default(),
            finished_jobs: Default::default(),
            job_actions: Default::default(),
            last_request: 0,
            next_handle: 0,
            sequence: 0,
            generation: format!("{time:x}-{serial:x}"),
        }
    }
}

impl Instance {
    pub fn buffer_handle(&mut self, buffer: usize) -> Result<String, Error> {
        Self::issue_handle(
            &mut self.buffers,
            &self.generation,
            &mut self.next_handle,
            "b",
            buffer,
            1024,
        )
    }

    pub fn pane_handle(&mut self, pane: usize) -> Result<String, Error> {
        Self::issue_handle(
            &mut self.panes,
            &self.generation,
            &mut self.next_handle,
            "p",
            pane,
            128,
        )
    }

    fn issue_handle(
        handles: &mut BTreeMap<String, usize>,
        generation: &str,
        next: &mut u64,
        prefix: &str,
        index: usize,
        limit: usize,
    ) -> Result<String, Error> {
        if let Some((handle, _)) = handles.iter().find(|(_, value)| **value == index) {
            return Ok(handle.clone());
        }
        if handles.len() >= limit {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "Editor handle limit reached",
            ));
        }
        *next += 1;
        let handle = format!("{prefix}:{generation}:{next}");
        handles.insert(handle.clone(), index);
        Ok(handle)
    }
}

pub const CAPABILITIES: &[&str] = &[
    "terminals",
    "external",
    "notifications",
    "processes",
    "providers",
    "interaction",
    "workspace",
    "jobs",
    "views",
    "text",
    "selections",
    "filesystem",
    "documents",
];

/// Validate framing independently of method dispatch so unknown methods receive
/// `unsupported`, while malformed known requests cannot be interpreted as others.
pub(crate) fn decode(bytes: &[u8]) -> anyhow::Result<super::ClientMessage> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get("type").and_then(|v| v.as_str()) == Some("request") {
        let object = value.as_object().unwrap();
        anyhow::ensure!(
            object.len() == 4 && object.contains_key("params"),
            "invalid request envelope"
        );
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing request ID"))?;
        let method = value
            .get("method")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing method"))?;
        anyhow::ensure!(
            id.len() <= 64 && method.len() <= 96,
            "invalid request ID or method"
        );
        if !matches!(
            method,
            "terminal.open"
                | "external.open"
                | "notification.publish"
                | "process.start"
                | "process.get"
                | "process.read"
                | "process.write"
                | "process.close"
                | "event.subscribe"
                | "event.unsubscribe"
                | "event.resync"
                | "provider.register"
                | "resource.open"
                | "resource.rebind"
                | "resource.inspect"
                | "buffer.list"
                | "buffer.open"
                | "buffer.save"
                | "buffer.close"
                | "buffer.create"
                | "ui.form"
                | "ui.prompt"
                | "ui.pick"
                | "ui.confirm"
                | "ui.dismiss"
                | "filesystem.stat"
                | "filesystem.list"
                | "filesystem.prepare"
                | "filesystem.apply"
                | "filesystem.cancel"
                | "filesystem.release"
                | "staging.create"
                | "staging.prepare"
                | "staging.close"
                | "pane.list"
                | "buffer.read"
                | "buffer.edit"
                | "buffer.snapshot.open"
                | "buffer.snapshot.read"
                | "buffer.snapshot.close"
                | "selection.get"
                | "selection.set"
                | "view.create"
                | "view.get"
                | "view.publish"
                | "view.query.set"
                | "view.patch"
                | "view.stage.open"
                | "view.stage.write"
                | "view.stage.commit"
                | "view.stage.close"
                | "view.snapshot.open"
                | "view.snapshot.read"
                | "view.snapshot.close"
                | "view.close"
                | "pane.show"
                | "workspace.info"
                | "job.create"
                | "job.get"
                | "job.update"
                | "job.finish"
                | "job.cancel"
        ) {
            return Ok(super::ClientMessage::Unsupported { id: id.to_owned() });
        }
    }
    if value.get("type").and_then(|v| v.as_str()) == Some("register") {
        let object = value.as_object().unwrap();
        anyhow::ensure!(object.len() == 6, "invalid registration envelope");
    }
    if value.get("type").and_then(|v| v.as_str()) == Some("response") {
        let object = value.as_object().unwrap();
        anyhow::ensure!(
            object.len() == 3 && (object.contains_key("result") ^ object.contains_key("error")),
            "invalid response envelope"
        );
    }
    Ok(super::ClientMessage::Application(serde_json::from_value(
        value,
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_fixtures_round_trip_through_the_wire() {
        use super::super::{observation as observe, view};
        let fixtures: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../docs/plugins/epoch2-fixtures.json")).unwrap();
        for fixture in fixtures.iter().filter(|f| f["direction"] == "plugin") {
            let message = &fixture["message"];
            let super::super::ClientMessage::Application(decoded) =
                decode(&serde_json::to_vec(message).unwrap()).unwrap()
            else {
                panic!()
            };
            assert_eq!(serde_json::to_value(decoded).unwrap(), *message);
        }
        let job = Job {
            job: "j:g:1".into(),
            title: "Download".into(),
            state: JobState::Running,
            progress: 0,
        };
        let messages = [
            HostMessage::Hello {
                version: VERSION,
                capabilities: CAPABILITIES.to_vec(),
                limits: Limits::default(),
            },
            HostMessage::Registered {
                commands: vec!["plugin.tasks.open".into()],
                capabilities: ["jobs".into()].into(),
                limits: Limits::default(),
            },
            HostMessage::Request {
                id: "h:1".into(),
                method: "command.invoke",
                params: Invocation {
                    arguments: Default::default(),
                    command: "open".into(),
                    context: CommandContext::Workspace,
                    pane: "p:g:1".into(),
                    selection_revision: "q:0".into(),
                    buffer: Some("b:g:2".into()),
                    buffer_revision: Some("r:0".into()),
                    view: None,
                    query_revision: None,
                    model_revision: None,
                    rows: vec![],
                },
            },
            HostMessage::Response {
                id: "p:1".into(),
                outcome: Response::Success {
                    result: ResultValue::Workspace {
                        name: "project".into(),
                    },
                },
            },
            HostMessage::Response {
                id: "p:2".into(),
                outcome: Response::Success {
                    result: ResultValue::Job(job.clone()),
                },
            },
            HostMessage::Response {
                id: "p:3".into(),
                outcome: Response::Failure {
                    error: Error::new(ErrorCode::NotFound, "Unknown job"),
                },
            },
            HostMessage::Event {
                sequence: "e:1".into(),
                event: "job.changed",
                data: job.into(),
            },
            HostMessage::Response {
                id: "p:100".into(),
                outcome: Response::Success {
                    result: ResultValue::Directory {
                        directory: "d:g:1".into(),
                        revision: "d:1".into(),
                        next: None,
                        entries: vec![super::super::filesystem::Entry {
                            entry: "n:1".into(),
                            name: "note.txt".into(),
                            kind: "file",
                            bytes: 5,
                        }],
                    },
                },
            },
            HostMessage::Response {
                id: "p:100".into(),
                outcome: Response::Success {
                    result: ResultValue::FilesystemPlan {
                        plan: "f:g:2".into(),
                        operations: vec!["create note.txt".into()],
                    },
                },
            },
            HostMessage::Response {
                id: "p:100".into(),
                outcome: Response::Success {
                    result: ResultValue::Opened {
                        buffer: "b:g:3".into(),
                        revision: "r:0".into(),
                    },
                },
            },
            HostMessage::Event {
                sequence: "e:2".into(),
                event: "filesystem.finished",
                data: EventData::FilesystemFinished(super::super::filesystem::Finished {
                    plan: "f:g:2".into(),
                    state: "succeeded",
                    applied: 1,
                    recovery: false,
                }),
            },
            HostMessage::Response {
                id: "p:200".into(),
                outcome: Response::Success {
                    result: ResultValue::Surface {
                        surface: "u:g:1".into(),
                    },
                },
            },
            HostMessage::InputRequest {
                id: "h:2".into(),
                method: "ui.submit",
                params: super::super::interaction::Submission {
                    sensitive: false,
                    surface: "u:g:1".into(),
                    accepted: true,
                    values: [(
                        "name".into(),
                        super::super::interaction::Value::Text("é".into()),
                    )]
                    .into(),
                },
            },
            HostMessage::Event {
                sequence: "e:12".into(),
                event: "filesystem.started",
                data: EventData::FilesystemStarted {
                    plan: "f:g:2".into(),
                    job: "j:g:3".into(),
                },
            },
            HostMessage::Response {
                id: "p:80".into(),
                outcome: Response::Success {
                    result: ResultValue::Stat(super::super::filesystem::Stat {
                        path: "notes.txt".into(),
                        kind: "file",
                        bytes: 5,
                        revision: "s:1".into(),
                    }),
                },
            },
            HostMessage::ResourceRequest {
                id: "h:3".into(),
                request: super::super::provider::Request::Stat {
                    job: "j:g:1".into(),
                    provider: "memory".into(),
                    key: "notes".into(),
                },
            },
            HostMessage::ResourceRequest {
                id: "h:4".into(),
                request: super::super::provider::Request::Read {
                    job: "j:g:1".into(),
                    provider: "memory".into(),
                    key: "notes".into(),
                    version: "v1".into(),
                    offset: 0,
                    limit: 131072,
                },
            },
            HostMessage::Event {
                sequence: "e:13".into(),
                event: "resource.opened",
                data: EventData::ResourceFinished(super::super::provider::Finished {
                    job: "j:g:1".into(),
                    buffer: Some("b:g:1".into()),
                    revision: Some("r:0".into()),
                    error: None,
                }),
            },
            HostMessage::ResourceRequest {
                id: "h:5".into(),
                request: super::super::provider::Request::WriteBegin {
                    job: "j:g:1".into(),
                    provider: "memory".into(),
                    key: "notes".into(),
                    expected_version: "v1".into(),
                    mode: super::super::provider::WriteMode::Conditional,
                    bytes: 5,
                    encoding: "utf-8",
                },
            },
            HostMessage::ResourceRequest {
                id: "h:6".into(),
                request: super::super::provider::Request::WriteChunk {
                    job: "j:g:1".into(),
                    upload: "upload-1".into(),
                    offset: 0,
                    text: "é猫".into(),
                },
            },
            HostMessage::ResourceRequest {
                id: "h:7".into(),
                request: super::super::provider::Request::WriteCommit {
                    job: "j:g:1".into(),
                    upload: "upload-1".into(),
                    expected_version: "v1".into(),
                    mode: super::super::provider::WriteMode::Conditional,
                },
            },
            HostMessage::ResourceRequest {
                id: "h:8".into(),
                request: super::super::provider::Request::WriteAbort {
                    job: "j:g:1".into(),
                    upload: Some("upload-1".into()),
                },
            },
            HostMessage::ResourceRequest {
                id: "h:9".into(),
                request: super::super::provider::Request::Reconcile {
                    job: "j:g:2".into(),
                    provider: "memory".into(),
                    key: "notes".into(),
                    previous_write: "j:g:1".into(),
                },
            },
            HostMessage::Event {
                sequence: "e:14".into(),
                event: "resource.saved",
                data: EventData::ResourceFinished(super::super::provider::Finished {
                    job: "j:g:1".into(),
                    buffer: Some("b:g:1".into()),
                    revision: Some("r:0".into()),
                    error: None,
                }),
            },
            HostMessage::Event {
                sequence: "e:15".into(),
                event: "resource.inspected",
                data: EventData::ResourceFinished(super::super::provider::Finished {
                    job: "j:g:3".into(),
                    buffer: Some("b:g:2".into()),
                    revision: Some("r:1".into()),
                    error: None,
                }),
            },
            HostMessage::ResourceRequest {
                id: "h:11".into(),
                request: super::super::provider::Request::WriteBegin {
                    job: "j:g:4".into(),
                    provider: "memory".into(),
                    key: "notes".into(),
                    expected_version: "v1".into(),
                    mode: super::super::provider::WriteMode::ConfirmedBestEffort,
                    bytes: 5,
                    encoding: "utf-8",
                },
            },
            HostMessage::ResourceRequest {
                id: "h:12".into(),
                request: super::super::provider::Request::WriteCommit {
                    job: "j:g:4".into(),
                    upload: "upload-2".into(),
                    expected_version: "v1".into(),
                    mode: super::super::provider::WriteMode::ConfirmedBestEffort,
                },
            },
            HostMessage::Event {
                sequence: "e:16".into(),
                event: "resource.released",
                data: EventData::ResourceReleased {
                    job: "j:g:4".into(),
                },
            },
            HostMessage::Response {
                id: "p:701".into(),
                outcome: Response::Success {
                    result: ResultValue::Staging {
                        staging: "s:g:1".into(),
                        path: "/runtime/cache/downloads/generation/stage-1".into(),
                    },
                },
            },
            HostMessage::Response {
                id: "p:400".into(),
                outcome: Response::Success {
                    result: ResultValue::Observation(super::super::observation::Baseline {
                        subscription: "o:g:1".into(),
                        sequence: "e:1".into(),
                        sources: vec![observation_fixture(false)],
                    }),
                },
            },
            HostMessage::Event {
                sequence: "e:2".into(),
                event: "event.changed",
                data: EventData::Observation(super::super::observation::Change {
                    subscription: "o:g:1".into(),
                    sources: vec![observation_fixture(false)],
                    coalesced: 2,
                }),
            },
            HostMessage::Event {
                sequence: "e:3".into(),
                event: "event.closed",
                data: EventData::Observation(super::super::observation::Change {
                    subscription: "o:g:1".into(),
                    sources: vec![observation_fixture(true)],
                    coalesced: 0,
                }),
            },
            HostMessage::Event {
                sequence: "e:4".into(),
                event: "event.resync_required",
                data: EventData::ObservationResync(super::super::observation::ResyncRequired {
                    subscription: "o:g:1".into(),
                }),
            },
            HostMessage::ValidationRequest {
                id: "h:21".into(),
                method: "ui.validate",
                params: super::super::interaction::ValidationRequest {
                    surface: "u:g:1".into(),
                    revision: "i:1".into(),
                    fields: vec!["name".into()],
                    values: [(
                        "name".into(),
                        super::super::interaction::Value::Text("example".into()),
                    )]
                    .into(),
                },
            },
            HostMessage::Event {
                sequence: "e:5".into(),
                event: "ui.validation_cancelled",
                data: EventData::ValidationCancelled(
                    super::super::interaction::ValidationCancelled {
                        request: "h:21".into(),
                        surface: "u:g:1".into(),
                        revision: "i:1".into(),
                    },
                ),
            },
            HostMessage::Response {
                id: "p:900".into(),
                outcome: Response::Success {
                    result: ResultValue::ViewInfo {
                        view: "v:g:1".into(),
                        revision: "m:2".into(),
                        bytes: 1_500_000,
                        rows: 1,
                        query: None,
                    },
                },
            },
            HostMessage::Response {
                id: "p:902".into(),
                outcome: Response::Success {
                    result: ResultValue::ViewStage {
                        stage: "vs:g:1".into(),
                        bytes: 2,
                    },
                },
            },
            HostMessage::Response {
                id: "p:903".into(),
                outcome: Response::Success {
                    result: ResultValue::ViewOffset { offset: 2 },
                },
            },
            HostMessage::Response {
                id: "p:906".into(),
                outcome: Response::Success {
                    result: ResultValue::ViewSnapshot {
                        snapshot: "vr:g:1".into(),
                        revision: "m:2".into(),
                        bytes: 2,
                    },
                },
            },
            HostMessage::Response {
                id: "p:907".into(),
                outcome: Response::Success {
                    result: ResultValue::ViewChunk {
                        offset: 0,
                        text: "{}".into(),
                        eof: true,
                    },
                },
            },
            HostMessage::Response {
                id: "p:950".into(),
                outcome: Response::Success {
                    result: ResultValue::ViewQuery {
                        view: "v:g:1".into(),
                        revision: "m:1".into(),
                        query: view::Query {
                            revision: "qv:1".into(),
                            text: "Topic 01".into(),
                            pending: true,
                        },
                    },
                },
            },
            HostMessage::Event {
                sequence: "e:950".into(),
                event: "event.changed",
                data: EventData::Observation(observe::Change {
                    subscription: "o:g:1".into(),
                    coalesced: 0,
                    sources: vec![observe::SourceState {
                        source: observe::Source::View {
                            view: "v:g:1".into(),
                        },
                        revision: "o:1".into(),
                        state: observe::Snapshot::View {
                            revision: "m:1".into(),
                            query: Some(view::Query {
                                revision: "qv:1".into(),
                                text: "Topic 01".into(),
                                pending: true,
                            }),
                        },
                    }],
                }),
            },
            HostMessage::Event {
                sequence: "e:951".into(),
                event: "event.changed",
                data: EventData::Observation(observe::Change {
                    subscription: "o:g:1".into(),
                    coalesced: 0,
                    sources: vec![observe::SourceState {
                        source: observe::Source::Viewport {
                            view: "v:g:1".into(),
                            pane: "n:g:1".into(),
                        },
                        revision: "o:2".into(),
                        state: observe::Snapshot::Viewport {
                            model_revision: Some("m:1".into()),
                            visible: true,
                            top: Some("row".into()),
                            bottom: Some("row".into()),
                        },
                    }],
                }),
            },
            HostMessage::Event {
                sequence: "e:952".into(),
                event: "event.action",
                data: EventData::ObservationAction(observe::ActionEvent {
                    subscription: "o:g:1".into(),
                    source: observe::Source::ViewActions {
                        view: "v:g:1".into(),
                    },
                    revision: "o:3".into(),
                    action: observe::Action {
                        id: "a:1".into(),
                        request: "h:950".into(),
                        command: "toggle".into(),
                        pane: "n:g:1".into(),
                        model_revision: "m:1".into(),
                        query_revision: Some("qv:2".into()),
                        selection_revision: "q:1".into(),
                        selected_count: 1,
                    },
                }),
            },
            HostMessage::Request {
                id: "h:950".into(),
                method: "command.invoke",
                params: Invocation {
                    command: "toggle".into(),
                    context: CommandContext::View,
                    view: Some("v:g:1".into()),
                    model_revision: Some("m:1".into()),
                    query_revision: Some("qv:2".into()),
                    rows: vec!["row".into()],
                    arguments: Default::default(),
                    pane: "n:g:1".into(),
                    selection_revision: "q:1".into(),
                    buffer: Some("b:g:1".into()),
                    buffer_revision: Some("r:1".into()),
                },
            },
            HostMessage::Response {
                id: "p:952".into(),
                outcome: Response::Success {
                    result: ResultValue::View {
                        view: "v:g:1".into(),
                        revision: "m:2".into(),
                        query: Some(view::Query {
                            revision: "qv:2".into(),
                            text: "Topic 01".into(),
                            pending: false,
                        }),
                        model: std::sync::Arc::new(view::Model {
                            title: "Rows".into(),
                            purpose: view::Purpose::List,
                            rows: vec![view::Row {
                                id: "row".into(),
                                text: "Topic 01".into(),
                                role: view::Role::Ordinary,
                                ..Default::default()
                            }],
                            ..Default::default()
                        }),
                    },
                },
            },
            HostMessage::Response {
                id: "p:1000".into(),
                outcome: Response::Success {
                    result: ResultValue::Process(super::super::process::Info {
                        process: "pr:1".into(),
                        label: "Echo helper".into(),
                        state: super::super::process::State::Running,
                        stdout: super::super::process::Bounds { start: 0, end: 6 },
                        stderr: Default::default(),
                        capture_stderr: false,
                        stdin_closed: false,
                        output_truncated: false,
                        exit_code: None,
                        signal: None,
                    }),
                },
            },
            HostMessage::Response {
                id: "p:1002".into(),
                outcome: Response::Success {
                    result: ResultValue::ProcessRead(super::super::process::Read {
                        process: "pr:1".into(),
                        stream: super::super::process::Stream::Stdout,
                        offset: 0,
                        data: "aGVsbG8K".into(),
                        next: 6,
                        eof: false,
                    }),
                },
            },
            HostMessage::Response {
                id: "p:1003".into(),
                outcome: Response::Success {
                    result: ResultValue::ProcessWrite(super::super::process::Write {
                        process: "pr:1".into(),
                        written: 6,
                        stdin_closed: true,
                    }),
                },
            },
            HostMessage::Event {
                sequence: "e:1000".into(),
                event: "event.changed",
                data: EventData::Observation(observe::Change {
                    subscription: "o:g:1".into(),
                    coalesced: 0,
                    sources: vec![observe::SourceState {
                        source: observe::Source::Process {
                            process: "pr:1".into(),
                        },
                        revision: "o:2".into(),
                        state: observe::Snapshot::Process {
                            state: super::super::process::State::Exited,
                            stdout: super::super::process::Bounds { start: 2, end: 8 },
                            stderr: Default::default(),
                            stdin_closed: true,
                            output_truncated: true,
                            exit_code: Some(0),
                            signal: None,
                        },
                    }],
                }),
            },
            HostMessage::Response {
                id: "p:1101".into(),
                outcome: Response::Success {
                    result: ResultValue::TerminalOpened {
                        terminal: "t:1".into(),
                    },
                },
            },
        ];
        let expected = fixtures
            .iter()
            .filter(|f| f["direction"] == "host")
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), expected.len());
        for (message, fixture) in messages.into_iter().zip(expected) {
            assert_eq!(serde_json::to_value(message).unwrap(), fixture["message"]);
        }
    }

    fn observation_fixture(closed: bool) -> super::super::observation::SourceState {
        use super::super::observation::{Snapshot, Source, SourceState};
        SourceState {
            source: Source::Buffer {
                buffer: "b:g:1".into(),
            },
            revision: if closed { "o:2" } else { "o:1" }.into(),
            state: if closed {
                Snapshot::Closed {}
            } else {
                Snapshot::Buffer {
                    revision: "r:1".into(),
                    saved_revision: Some("r:1".into()),
                    name: "scratch".into(),
                    chars: 0,
                    read_only: false,
                    dirty: false,
                }
            },
        }
    }

    #[test]
    fn process_wire_rejects_ambiguous_operating_system_values() {
        for (method, params) in [
            (
                "process.start",
                serde_json::json!({"label":"Helper","executable":"helper","args":"--flag"}),
            ),
            (
                "process.start",
                serde_json::json!({"label":"Helper","executable":"helper","args":[null]}),
            ),
            (
                "process.start",
                serde_json::json!({"label":"Helper","executable":"helper","cwd":false}),
            ),
            (
                "process.start",
                serde_json::json!({"label":"Helper","executable":"helper","capture_stderr":1}),
            ),
            (
                "process.start",
                serde_json::json!({"label":"Helper","executable":"helper","shell":true}),
            ),
            (
                "process.read",
                serde_json::json!({"process":"pr:1","stream":"stdout","offset":-1,"limit":1}),
            ),
            (
                "process.read",
                serde_json::json!({"process":"pr:1","stream":"stdout","offset":0,"limit":1.5}),
            ),
            (
                "process.read",
                serde_json::json!({"process":"pr:1","stream":"stdout","offset":1e30,"limit":1}),
            ),
            (
                "process.write",
                serde_json::json!({"process":"pr:1","data":[65],"eof":false}),
            ),
            (
                "process.write",
                serde_json::json!({"process":"pr:1","data":"","eof":"true"}),
            ),
            (
                "process.close",
                serde_json::json!({"process":"pr:1","signal":9}),
            ),
        ] {
            let frame =
                serde_json::json!({"type":"request","id":"p:1","method":method,"params":params});
            assert!(
                decode(&serde_json::to_vec(&frame).unwrap()).is_err(),
                "{frame}"
            );
        }
    }

    #[test]
    fn staging_wire_requires_exact_request_fields_and_types() {
        let fixtures: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../docs/plugins/epoch2-fixtures.json")).unwrap();
        for fixture in fixtures.iter().filter(|fixture| {
            fixture["message"]["method"]
                .as_str()
                .is_some_and(|method| method.starts_with("staging."))
        }) {
            let message = &fixture["message"];
            for field in message["params"].as_object().unwrap().keys() {
                let mut missing = message.clone();
                missing["params"].as_object_mut().unwrap().remove(field);
                assert!(decode(&serde_json::to_vec(&missing).unwrap()).is_err());
            }
            let mut extra = message.clone();
            extra["params"]["path"] = serde_json::json!("/unissued");
            assert!(decode(&serde_json::to_vec(&extra).unwrap()).is_err());
        }
        for bytes in [
            serde_json::json!(-1),
            serde_json::json!(true),
            serde_json::json!("5"),
        ] {
            let message = serde_json::json!({
                "type": "request", "id": "p:701", "method": "staging.create",
                "params": {"job": "j:g:5", "bytes": bytes}
            });
            assert!(decode(&serde_json::to_vec(&message).unwrap()).is_err());
        }
    }

    #[test]
    fn wire_rejects_ambiguous_results_and_strict_request_fields() {
        assert!(decode(br#"{"type":"request","id":"p:1","method":"job.create","params":{"title":"Copy","deadline_seconds":60}}"#).is_ok());
        assert!(decode(br#"{"type":"response","id":"h:2","result":{"job":null}}"#).is_ok());
        for message in [
            r#"{"type":"request","id":"p:1","method":"resource.inspect","params":{"buffer":"b:g:1","expected_revision":"r:0"}}"#,
            r#"{"type":"request","id":"p:1","method":"resource.inspect","params":{"buffer":"b:g:1","expected_revision":"r:0","invocation":null}}"#,
            r#"{"type":"response","id":"h:2","result":{"job":null},"error":{"code":"stale","message":"Changed"}}"#,
            r#"{"type":"request","id":"p:1","method":"job.create","params":{"title":"Copy","deadline_seconds":60,"extra":true}}"#,
            r#"{"type":"request","id":"p:1","method":"workspace.info","params":{},"extra":true}"#,
        ] {
            assert!(decode(message.as_bytes()).is_err(), "{message}");
        }
        assert!(matches!(
            decode(br#"{"type":"request","id":"p:1","method":"future.method","params":{}}"#)
                .unwrap(),
            super::super::ClientMessage::Unsupported { .. }
        ));
    }
}
