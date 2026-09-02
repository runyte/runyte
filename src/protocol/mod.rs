// SPDX-License-Identifier: MPL-2.0

//! Private, versioned DTOs used only by bundled local clients.
//!
//! Workspace values deliberately do not implement serialization. Conversion
//! happens here so changing a core representation cannot silently change the
//! socket contract.

use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

mod frame;
mod input;

pub use frame::{
    EditorSnapshot, ExternalFileStatus, FrameId, HostFrame, SnapshotRow, TerminalDamageFrame,
};
pub use input::{FrameGeometry, InputEvent, PointerEvent, Rect};

use crate::app::{
    CommandOutcome as CoreCommandOutcome, PromptKind as CorePromptKind,
    SearchMode as CoreSearchMode,
};
use crate::workspace::{
    BufferContents as CoreBufferContents, BufferId as CoreBufferId,
    BufferMetadata as CoreBufferMetadata, BufferRevision as CoreBufferRevision,
    SessionPreview as CoreSessionPreview, SessionPreviewPane as CoreSessionPreviewPane,
    SessionPreviewPaneKind as CoreSessionPreviewPaneKind, WaitStatus as CoreWaitStatus,
    WaitToken as CoreWaitToken,
};

/// Moves whenever the wire surface gains a case a peer of the previous version
/// cannot serve, so that bundled installations which have drifted apart fail at
/// the handshake instead of part-way through a command. Version 4 added the
/// `join-delimiter` prompt: a host still running the previous binary owns the
/// keymap, and would answer `Space p j` with an unbound-key error rather than
/// the prompt the newer client is waiting to render. Version 6 replaces the
/// two numeric-setting prompt cases with one registry-keyed value prompt.
/// Version 7 adds content alignment to a pane frame: a host still running the
/// previous binary sends no indent and no padding rows, and a newer client
/// would draw a centred page against the left edge of the pane. Version 8 adds
/// mixed-length jump-label roles, active-pane jump dimming, and the immediate
/// label colour to frames. Version 9 gives jump-dimmed text its own theme role.
/// Version 10 adds fuzzy-grep match roles and preserves semantic match spans in
/// picker previews. Version 11 adopts the UI vocabulary in frame geometry,
/// carries notification counts and row severities, interaction-line fields,
/// and semantic notification colours. Version 12 adds matched full-text picker
/// previews for commit search. Version 13 adds persistent host naming for the
/// bundled lifecycle-management client. Version 14 carries the directory chosen
/// by `:quit-here` on the exit response: the editor that chooses it runs in
/// the host, while the file a shell wrapper reads belongs to the client, so a
/// host still running the previous binary would exit without ever reporting
/// where the shell should go. Version 15 lets a client report a message for the
/// editor to show, which is how a client that could not reach a destination
/// workspace explains why it stayed on the one it was already attached to.
/// Version 16 carries the editor working directory with a workspace-switch
/// selector, so relative paths retain `:cd` semantics across the host/client
/// boundary without converting exact names or ID prefixes into paths. Version
/// 17 replaces active-buffer identity in the global status snapshot with the
/// editor working directory; older clients would otherwise continue to label
/// that value as a file.
/// Version 18 combines those additions with semantic overlay purpose, input,
/// action, layout, row-bound fields, and changed-line count roles from the
/// editor protocol.
/// Version 19 reports how many buffers hold unsaved work in the health
/// response. It is the host's own answer rather than a count a client derives
/// from the buffer list, so a listing cannot call a workspace clean that the
/// host would refuse to stop; a scratch buffer is excluded from both.
/// Version 20 gives a pane frame its terminal cells. The field is required
/// rather than optional on the wire, so a host still running the previous
/// binary sends a frame a newer client cannot deserialize at all — not a frame
/// it would draw wrongly. That is a case the handshake has to reject, which is
/// what this number is for. Version 21 makes terminal and wait-request counts
/// part of host health and adds an explicit force-shutdown request. Older
/// clients must not mistake a host with a live terminal for a clean one.
/// Version 22 adds revisioned terminal row damage and explicit full-frame
/// resynchronization after a stale base.
/// Version 23 adds the contextual catalog-rename prompt returned by the
/// persistent-session manager's action overlay.
/// Version 24 adds bounded repetition counts to pointer requests so attached
/// clients can coalesce wheel bursts without losing their semantic distance.
/// Version 25 adds terminal jump-label highlight roles. An older client cannot
/// deserialize those enum variants and must be rejected by the handshake.
/// Version 26 renames the persistent-session rename prompt on the wire so
/// bundled clients cannot present the superseded workspace terminology.
/// Version 27 adds the command-mode pane dimming flag and the fourth mode
/// cursor colour. An older client would draw undimmed panes and has no field
/// to put the Command caret colour in.
/// Version 28 separates live terminal Normal from review and makes directional
/// pane focus leave Terminal Insert in live Normal. The workspace host owns
/// key dispatch, so an older host would otherwise accept a newer client while
/// continuing to execute the superseded two-step motions.
/// Version 29 carries the workspace's session number in the status snapshot
/// and adds the session-number prompt. An older client cannot deserialize the
/// prompt. The snapshot field defaults on the wire, so the shape alone would
/// survive an older host, but the handshake still rejects one: a client
/// drawing `[S1]` beside a workspace whose host cannot report a number would
/// show every session as unnumbered rather than showing nothing, which is a
/// wrong answer rather than a missing one.
/// Version 30 carries contextual row availability in owned overlays so an
/// attached client dims unavailable key-hint namespaces just like standalone.
/// Version 31 carries per-row dimming beside that availability, so a stopped
/// session in an attached client's list reads as dormant rather than as one
/// more running row. An older client has no field to put it in and would
/// draw every session at full weight.
/// Version 32 makes every pane-focus route enter live Insert on a terminal
/// destination. The host owns this mode transition, so an older host would
/// otherwise retain the superseded directional-focus behavior.
/// Version 33 requests disambiguated Ctrl-key reporting from macOS terminals.
/// Physical terminal input is client-owned, so an older attached client would
/// otherwise retain the superseded two-step terminal pane motions even when
/// the workspace host was built with the correction.
/// Version 34 carries the maximized presentation in a pane title, so an
/// attached client marks `:zen` and `:fullscreen` the way standalone does. The
/// field is required rather than optional on the wire, so a host still running
/// the previous binary sends a title a newer client cannot deserialize at all.
/// Version 35 adds a bounded semantic session preview for the persistent-
/// session manager. A previous host cannot answer the new control request, so
/// the handshake rejects it before the manager waits for a response it will
/// never receive.
/// Version 36 preserves a terminal's captured review when pane focus returns
/// to it. The host owns focus and mode transitions, so an older host would
/// otherwise discard review requested through a newer attached client.
/// Version 37 carries the whitespace theme role and semantic whitespace runs.
/// An older attached client would otherwise render the markers as ordinary
/// syntax-coloured text or fail to deserialize the resolved theme.
/// Version 38 reports how many buffers a host holds open, beside the count of
/// those holding unsaved work. The session manager states the number as a fact
/// about the session, and a host still running the previous binary has no
/// answer to give: every such session would read `Buffers: 0`, which is a
/// wrong answer rather than a missing one. Version 39 adds Replace mode and
/// its resolved caret colour to frames; older peers cannot deserialize either
/// required enum or theme value. Version 40 carries semantic external-file
/// state in pane titles and the global status line; an older attached client
/// would otherwise omit `[STALE]` and hide a conflict retained by the host.
/// Version 41 carries a picker row's pinned trailing column so an older client
/// cannot silently clip the session manager's last-active value.
/// Version 42 carries non-selectable picker column labels so an older attached
/// client cannot silently omit the session manager's headings.
/// Version 43 splits the palette's command colour out of `accent` and carries
/// it as its own required theme value; an older peer has no field for it and
/// cannot deserialize the resolved theme at all.
/// Version 44 adds the `terminal-rename` prompt, which the terminal manager's
/// Rename action opens against a listed session rather than an attached one. A
/// host still running the previous binary owns key dispatch, so it would
/// answer that action by attaching the terminal to the active pane and opening
/// the command prompt instead, and it has no case for the new prompt on the
/// wire.
pub const VERSION: u32 = 44;
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MAX_PATHS: usize = 32;
pub const MAX_PATH_BYTES: usize = 32 * 1024;
pub const MAX_COMMAND_BYTES: usize = 16 * 1024;
pub const MAX_NOTIFICATION_BYTES: usize = 16 * 1024;
pub const MAX_INPUT_TEXT_BYTES: usize = crate::input::MAX_TEXT_INPUT_BYTES;
pub const MAX_POINTER_REPETITIONS: u16 = 256;
pub const MAX_FEATURE_GROUPS: usize = 16;
pub const MAX_TRANSACTION_CHANGES: usize = 4096;
pub const MAX_TRANSACTION_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct BufferId(u64);

impl BufferId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<CoreBufferId> for BufferId {
    fn from(value: CoreBufferId) -> Self {
        Self(value.get())
    }
}

impl From<BufferId> for CoreBufferId {
    fn from(value: BufferId) -> Self {
        Self::from_raw(value.0)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct BufferRevision(u64);

impl BufferRevision {
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<CoreBufferRevision> for BufferRevision {
    fn from(value: CoreBufferRevision) -> Self {
        Self(value.get())
    }
}

impl From<BufferRevision> for CoreBufferRevision {
    fn from(value: BufferRevision) -> Self {
        Self::from_raw(value.0)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BufferMetadata {
    pub id: BufferId,
    pub revision: BufferRevision,
    pub path_bytes: Option<Vec<u8>>,
    pub name: String,
    pub dirty: bool,
    pub read_only: bool,
    pub closed: bool,
}

impl From<CoreBufferMetadata> for BufferMetadata {
    fn from(value: CoreBufferMetadata) -> Self {
        Self {
            id: value.id.into(),
            revision: value.revision.into(),
            path_bytes: value.path.map(|path| encode_path(&path)),
            name: value.name,
            dirty: value.dirty,
            read_only: value.read_only,
            closed: value.closed,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BufferContents {
    pub metadata: BufferMetadata,
    pub text: String,
    pub truncated: bool,
}

impl From<CoreBufferContents> for BufferContents {
    fn from(value: CoreBufferContents) -> Self {
        Self {
            metadata: value.metadata.into(),
            text: value.text,
            truncated: value.truncated,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionPreview {
    pub layout_panes: usize,
    pub panes: Vec<SessionPreviewPane>,
    pub omitted_panes: usize,
    pub other_resources: Vec<String>,
    pub omitted_resources: usize,
}

impl From<CoreSessionPreview> for SessionPreview {
    fn from(value: CoreSessionPreview) -> Self {
        Self {
            layout_panes: value.layout_panes,
            panes: value.panes.into_iter().map(Into::into).collect(),
            omitted_panes: value.omitted_panes,
            other_resources: value.other_resources,
            omitted_resources: value.omitted_resources,
        }
    }
}

impl From<SessionPreview> for CoreSessionPreview {
    fn from(value: SessionPreview) -> Self {
        Self {
            layout_panes: value.layout_panes,
            panes: value.panes.into_iter().map(Into::into).collect(),
            omitted_panes: value.omitted_panes,
            other_resources: value.other_resources,
            omitted_resources: value.omitted_resources,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionPreviewPane {
    pub active: bool,
    pub title: String,
    pub kind: SessionPreviewPaneKind,
    pub start_line: Option<usize>,
    pub lines: Vec<String>,
}

impl From<CoreSessionPreviewPane> for SessionPreviewPane {
    fn from(value: CoreSessionPreviewPane) -> Self {
        Self {
            active: value.active,
            title: value.title,
            kind: value.kind.into(),
            start_line: value.start_line,
            lines: value.lines,
        }
    }
}

impl From<SessionPreviewPane> for CoreSessionPreviewPane {
    fn from(value: SessionPreviewPane) -> Self {
        Self {
            active: value.active,
            title: value.title,
            kind: value.kind.into(),
            start_line: value.start_line,
            lines: value.lines,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SessionPreviewPaneKind {
    Buffer { dirty: bool, read_only: bool },
    Terminal { live: bool },
}

impl From<CoreSessionPreviewPaneKind> for SessionPreviewPaneKind {
    fn from(value: CoreSessionPreviewPaneKind) -> Self {
        match value {
            CoreSessionPreviewPaneKind::Buffer { dirty, read_only } => {
                Self::Buffer { dirty, read_only }
            }
            CoreSessionPreviewPaneKind::Terminal { live } => Self::Terminal { live },
        }
    }
}

impl From<SessionPreviewPaneKind> for CoreSessionPreviewPaneKind {
    fn from(value: SessionPreviewPaneKind) -> Self {
        match value {
            SessionPreviewPaneKind::Buffer { dirty, read_only } => {
                Self::Buffer { dirty, read_only }
            }
            SessionPreviewPaneKind::Terminal { live } => Self::Terminal { live },
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct WaitToken(u64);

impl WaitToken {
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for WaitToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<CoreWaitToken> for WaitToken {
    fn from(value: CoreWaitToken) -> Self {
        Self(value.get())
    }
}

impl From<WaitToken> for CoreWaitToken {
    fn from(value: WaitToken) -> Self {
        Self::new(value.0)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum WaitStatus {
    Pending {
        buffers: Vec<BufferId>,
        remaining: Vec<BufferId>,
    },
    Completed,
    Cancelled {
        reason: String,
    },
}

impl From<CoreWaitStatus> for WaitStatus {
    fn from(value: CoreWaitStatus) -> Self {
        match value {
            CoreWaitStatus::Pending { buffers, remaining } => Self::Pending {
                buffers: buffers.into_iter().map(Into::into).collect(),
                remaining: remaining.into_iter().map(Into::into).collect(),
            },
            CoreWaitStatus::Completed => Self::Completed,
            CoreWaitStatus::Cancelled { reason } => Self::Cancelled { reason },
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub enum ClientRequest {
    Hello {
        protocol: u32,
        features: Vec<FeatureGroup>,
        /// The workspace's project root. The wire key keeps its original
        /// spelling so a new client and an already-running host stay
        /// compatible; only the Rust name is corrected.
        #[serde(rename = "workspace_root_bytes")]
        project_root_bytes: Vec<u8>,
        client_kind: ClientKind,
        client_version: String,
        role: ClientRole,
        geometry: FrameGeometry,
        /// Whether this client can hand a chosen directory to its shell, which
        /// is what makes `:quit-here` meaningful. A host outlives its clients
        /// and each one is launched separately, so the capability travels with
        /// the client rather than being a property of the host.
        #[serde(default)]
        directory_handoff: bool,
    },
    Input {
        event: InputEvent,
        repeated: bool,
    },
    Invoke {
        command: CommandRequest,
    },
    /// A message the client needs the person to see. The attached client draws
    /// only host frames and owns no status line, so anything it discovers on its
    /// own — a destination workspace it could not reach, for instance — has to
    /// be reported through the editor that is on screen.
    Notify {
        message: String,
    },
    Health,
    SessionPreview,
    ListBuffers,
    ReadBuffer {
        buffer: BufferId,
    },
    OpenBuffers {
        paths: Vec<Vec<u8>>,
        activate: bool,
    },
    ApplyTransaction {
        buffer: BufferId,
        expected: BufferRevision,
        changes: Vec<TransportChange>,
    },
    SaveBuffer {
        buffer: BufferId,
    },
    CloseBuffer {
        buffer: BufferId,
        discard: bool,
    },
    CreateWait {
        paths: Vec<Vec<u8>>,
    },
    AttachWait {
        token: WaitToken,
    },
    WaitStatus {
        token: WaitToken,
    },
    CompleteWaitBuffer {
        token: WaitToken,
        buffer: BufferId,
    },
    CancelWait {
        token: WaitToken,
    },
    RenameHost {
        name: String,
    },
    Pointer {
        event: PointerEvent,
        frame: FrameId,
        repetitions: u16,
    },
    Resize {
        geometry: FrameGeometry,
    },
    Resynchronize,
    Detach,
    Shutdown,
    ForceShutdown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    pub name: String,
    pub argument: Option<String>,
    pub frame: FrameId,
    pub buffer: BufferId,
    pub revision: BufferRevision,
}

impl CommandRequest {
    pub fn at(
        name: impl Into<String>,
        frame: FrameId,
        buffer: BufferId,
        revision: BufferRevision,
    ) -> Self {
        Self {
            name: name.into(),
            argument: None,
            frame,
            buffer,
            revision,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", content = "value", rename_all = "kebab-case")]
pub enum CommandOutcome {
    Completed,
    Status(String),
    UserError(String),
    Unavailable(String),
    Confirmation(String),
    Prompt(PromptKind),
    AsynchronousRequest(Option<String>),
}

impl From<CoreCommandOutcome> for CommandOutcome {
    fn from(value: CoreCommandOutcome) -> Self {
        match value {
            CoreCommandOutcome::Completed => Self::Completed,
            CoreCommandOutcome::Status(message) => Self::Status(message),
            CoreCommandOutcome::UserError(message) => Self::UserError(message),
            CoreCommandOutcome::Unavailable(message) => Self::Unavailable(message),
            CoreCommandOutcome::Confirmation(message) => Self::Confirmation(message),
            CoreCommandOutcome::Prompt(kind) => Self::Prompt(kind.into()),
            CoreCommandOutcome::AsynchronousRequest(message) => Self::AsynchronousRequest(message),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum PromptKind {
    Command,
    Search(SearchMode),
    TerminalSearch(SearchMode),
    SearchForward,
    SearchBackward,
    GlobalSearch(SearchMode),
    FilterSelections { keep: bool },
    Rename,
    SessionRename,
    SessionNumber,
    TerminalRename,
    ExternalProgram,
    NewBranch,
    NewWorktreeBranch,
    WorktreeDestination,
    JoinDelimiter,
    SettingValue(String),
    FinderPath,
}

impl From<CorePromptKind> for PromptKind {
    fn from(value: CorePromptKind) -> Self {
        match value {
            CorePromptKind::Command => Self::Command,
            CorePromptKind::Search(mode) => Self::Search(mode.into()),
            CorePromptKind::TerminalSearch(mode) => Self::TerminalSearch(mode.into()),
            CorePromptKind::SearchForward => Self::SearchForward,
            CorePromptKind::SearchBackward => Self::SearchBackward,
            CorePromptKind::GlobalSearch(mode) => Self::GlobalSearch(mode.into()),
            CorePromptKind::FilterSelections { keep } => Self::FilterSelections { keep },
            CorePromptKind::Rename => Self::Rename,
            CorePromptKind::SessionRename => Self::SessionRename,
            CorePromptKind::SessionNumber => Self::SessionNumber,
            CorePromptKind::TerminalRename => Self::TerminalRename,
            CorePromptKind::ExternalProgram => Self::ExternalProgram,
            CorePromptKind::NewBranch => Self::NewBranch,
            CorePromptKind::NewWorktreeBranch => Self::NewWorktreeBranch,
            CorePromptKind::WorktreeDestination => Self::WorktreeDestination,
            CorePromptKind::JoinDelimiter => Self::JoinDelimiter,
            CorePromptKind::SettingValue(setting) => {
                Self::SettingValue(setting.descriptor().key.to_owned())
            }
            CorePromptKind::FinderPath => Self::FinderPath,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchMode {
    Insensitive,
    Sensitive,
    Regex,
}

impl From<CoreSearchMode> for SearchMode {
    fn from(value: CoreSearchMode) -> Self {
        match value {
            CoreSearchMode::Insensitive => Self::Insensitive,
            CoreSearchMode::Sensitive => Self::Sensitive,
            CoreSearchMode::Regex => Self::Regex,
        }
    }
}

impl ClientRequest {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Hello {
                features,
                project_root_bytes,
                client_version,
                geometry,
                ..
            } => {
                require(
                    !features.is_empty() && features.len() <= MAX_FEATURE_GROUPS,
                    "invalid feature count",
                )?;
                require(
                    !project_root_bytes.is_empty() && project_root_bytes.len() <= MAX_PATH_BYTES,
                    "invalid workspace identity length",
                )?;
                require(
                    !client_version.is_empty() && client_version.len() <= 128,
                    "invalid client version length",
                )?;
                geometry.validate()
            }
            Self::Input { event, .. } => match event {
                InputEvent::Text(text) => require(
                    text.len() <= MAX_INPUT_TEXT_BYTES,
                    "text input exceeds the protocol limit",
                ),
                InputEvent::Key(key) => require(
                    key.modifiers & !0x3f == 0,
                    "key input contains unsupported modifier bits",
                ),
                InputEvent::Pointer(pointer) => require(
                    pointer.modifiers & !0x3f == 0,
                    "pointer input contains unsupported modifier bits",
                ),
            },
            Self::Pointer {
                event, repetitions, ..
            } => {
                require(
                    event.modifiers & !0x3f == 0,
                    "pointer input contains unsupported modifier bits",
                )?;
                require(
                    (1..=MAX_POINTER_REPETITIONS).contains(repetitions),
                    "pointer repetition count is outside the protocol limit",
                )?;
                require(
                    *repetitions == 1
                        || matches!(
                            event.kind,
                            input::PointerEventKind::ScrollUp
                                | input::PointerEventKind::ScrollDown
                                | input::PointerEventKind::ScrollLeft
                                | input::PointerEventKind::ScrollRight
                        ),
                    "only wheel pointer input may be repeated",
                )
            }
            Self::Invoke { command } => {
                require(
                    !command.name.is_empty()
                        && command.name.len() <= 128
                        && !command.name.chars().any(char::is_whitespace),
                    "command identity is invalid",
                )?;
                require(
                    command
                        .argument
                        .as_ref()
                        .is_none_or(|argument| argument.len() <= MAX_COMMAND_BYTES),
                    "command argument exceeds the protocol limit",
                )
            }
            Self::Notify { message } => require(
                !message.is_empty() && message.len() <= MAX_NOTIFICATION_BYTES,
                "client notification exceeds the protocol limit",
            ),
            Self::OpenBuffers { paths, .. } | Self::CreateWait { paths } => {
                require(
                    !paths.is_empty() && paths.len() <= MAX_PATHS,
                    "path request must contain 1 to 32 paths",
                )?;
                require(
                    paths
                        .iter()
                        .all(|path| !path.is_empty() && path.len() <= MAX_PATH_BYTES),
                    "path identity exceeds the protocol limit",
                )
            }
            Self::ApplyTransaction { changes, .. } => {
                require(
                    !changes.is_empty() && changes.len() <= MAX_TRANSACTION_CHANGES,
                    "transaction must contain 1 to 4096 changes",
                )?;
                require(
                    changes.iter().all(|change| change.from <= change.to),
                    "transaction ranges must be forward and half-open",
                )?;
                require(
                    changes.windows(2).all(|pair| pair[0].to <= pair[1].from),
                    "transaction ranges must be ordered and non-overlapping",
                )?;
                let text_bytes = changes.iter().try_fold(0_usize, |total, change| {
                    total.checked_add(change.text.len())
                });
                require(
                    text_bytes.is_some_and(|bytes| bytes <= MAX_TRANSACTION_TEXT_BYTES),
                    "transaction text exceeds the protocol limit",
                )
            }
            Self::RenameHost { name } => require(
                !name.is_empty()
                    && name == name.trim()
                    && name.len() <= 64
                    && !name.chars().any(char::is_control),
                "host name is invalid",
            ),
            Self::Resize { geometry } => geometry.validate(),
            _ => Ok(()),
        }
    }
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeatureGroup {
    Snapshots,
    Input,
    Control,
    Buffers,
    Wait,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransportChange {
    pub from: usize,
    pub to: usize,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClientKind {
    Tui,
    Control,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClientRole {
    Interactive,
    Control,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum HostResponse {
    Welcome {
        protocol: u32,
        pid: u32,
        features: Vec<FeatureGroup>,
        host_version: String,
    },
    Refused {
        message: String,
    },
    Frame {
        frame: Box<HostFrame>,
    },
    TerminalDamage {
        damage: Box<TerminalDamageFrame>,
    },
    Detached {
        /// Where `:quit-here` asked the invoking shell to go, when this response
        /// came from that command. The client owns the file a shell wrapper
        /// reads, so the host reports the directory rather than writing it.
        directory_bytes: Option<Vec<u8>>,
    },
    ShuttingDown,
    HostRenamed {
        name: String,
    },
    SwitchWorkspace {
        selector_bytes: Vec<u8>,
        working_directory_bytes: Vec<u8>,
    },
    Error {
        message: String,
    },
    Health {
        protocol: u32,
        pid: u32,
        interactive_attached: bool,
        /// Live buffers holding unsaved work, which is what makes this
        /// workspace refuse to stop. The scratch buffer is not one of them.
        unsaved_buffers: usize,
        /// Every buffer the host holds open, unsaved or not. Reported rather
        /// than derived from the buffer list so a listing and the host agree
        /// about what this session is holding without transferring it.
        open_buffers: usize,
        /// Pending editor-wait requests protected by the same lifecycle rule.
        pending_wait_requests: usize,
        /// Children that are still running. Exited retained screens are not
        /// live state and do not keep a clean host alive.
        live_terminals: usize,
        /// All retained terminal sessions, including exited screens.
        terminal_sessions: usize,
    },
    SessionPreview {
        preview: SessionPreview,
    },
    CommandResult {
        outcome: CommandOutcome,
    },
    Buffers {
        buffers: Vec<BufferMetadata>,
    },
    Buffer {
        buffer: BufferContents,
    },
    Opened {
        buffers: Vec<BufferId>,
    },
    TransactionApplied {
        buffer: BufferId,
        revision: BufferRevision,
    },
    StaleRevision {
        buffer: BufferId,
        expected: BufferRevision,
        actual: BufferRevision,
    },
    Saved {
        buffer: BufferId,
        revision: BufferRevision,
    },
    Closed {
        buffer: BufferId,
    },
    WaitCreated {
        token: WaitToken,
        buffers: Vec<BufferId>,
        interactive_attached: bool,
    },
    WaitState {
        token: WaitToken,
        status: WaitStatus,
        interactive_attached: bool,
    },
}

pub fn validate_welcome(response: &HostResponse, interactive: bool) -> Result<(), String> {
    let HostResponse::Welcome {
        protocol,
        features,
        host_version,
        ..
    } = response
    else {
        return Err("workspace host did not send a welcome response".to_owned());
    };
    require(
        *protocol == VERSION,
        "workspace host welcome uses an incompatible protocol",
    )?;
    let expected: &[FeatureGroup] = if interactive {
        &[
            FeatureGroup::Snapshots,
            FeatureGroup::Input,
            FeatureGroup::Buffers,
            FeatureGroup::Wait,
        ]
    } else {
        &[
            FeatureGroup::Control,
            FeatureGroup::Buffers,
            FeatureGroup::Wait,
        ]
    };
    require(
        features.as_slice() == expected,
        "workspace host welcome has an incompatible feature set",
    )?;
    require(
        !host_version.is_empty() && host_version.len() <= 128,
        "workspace host welcome has an invalid version",
    )
}

pub fn encode_path(path: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

pub fn decode_path(bytes: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_and_request_bounds_are_explicit() {
        assert_eq!(VERSION, 44);
        let oversized_command = ClientRequest::Invoke {
            command: CommandRequest {
                name: "open".to_owned(),
                argument: Some("x".repeat(MAX_COMMAND_BYTES + 1)),
                frame: FrameId::from_raw(1),
                buffer: BufferId(1),
                revision: BufferRevision(1),
            },
        };
        assert!(oversized_command.validate().is_err());
        let too_many_paths = ClientRequest::CreateWait {
            paths: vec![b"file".to_vec(); MAX_PATHS + 1],
        };
        assert!(too_many_paths.validate().is_err());
        let oversized_transaction = ClientRequest::ApplyTransaction {
            buffer: BufferId(1),
            expected: BufferRevision(1),
            changes: vec![TransportChange {
                from: 0,
                to: 0,
                text: "x".repeat(MAX_TRANSACTION_TEXT_BYTES + 1),
            }],
        };
        assert!(oversized_transaction.validate().is_err());
        assert!(
            ClientRequest::Notify {
                message: "x".repeat(MAX_NOTIFICATION_BYTES + 1),
            }
            .validate()
            .is_err()
        );
        assert!(
            ClientRequest::Resize {
                geometry: FrameGeometry {
                    screen: Rect {
                        width: 4096,
                        height: 4096,
                        ..Rect::default()
                    },
                    ..FrameGeometry::default()
                },
            }
            .validate()
            .is_err()
        );
        let overflowing = Rect {
            x: u16::MAX,
            width: 1,
            ..Rect::default()
        };
        assert!(
            ClientRequest::Resize {
                geometry: FrameGeometry {
                    screen: overflowing,
                    editor: overflowing,
                    global_status_line: overflowing,
                    interaction_line: overflowing,
                },
            }
            .validate()
            .is_err()
        );
        let maximum = FrameGeometry {
            screen: Rect {
                width: 256,
                height: 128,
                ..Rect::default()
            },
            ..FrameGeometry::default()
        };
        assert_eq!(
            usize::from(256_u16) * usize::from(128_u16),
            input::FrameGeometry::MAX_CELLS
        );
        assert!(
            ClientRequest::Resize { geometry: maximum }
                .validate()
                .is_ok()
        );
        assert!(
            ClientRequest::Resize {
                geometry: FrameGeometry {
                    screen: Rect {
                        width: 257,
                        height: 128,
                        ..Rect::default()
                    },
                    ..FrameGeometry::default()
                },
            }
            .validate()
            .is_err()
        );
        assert!(
            ClientRequest::Resize {
                geometry: FrameGeometry {
                    screen: Rect {
                        width: 80,
                        height: 24,
                        ..Rect::default()
                    },
                    editor: Rect {
                        x: 79,
                        width: 2,
                        ..Rect::default()
                    },
                    ..FrameGeometry::default()
                },
            }
            .validate()
            .is_err()
        );
        assert!(
            ClientRequest::RenameHost {
                name: " x".to_owned(),
            }
            .validate()
            .is_err()
        );
        assert!(
            ClientRequest::RenameHost {
                name: "x".repeat(65),
            }
            .validate()
            .is_err()
        );

        let wheel = input::PointerEvent {
            kind: input::PointerEventKind::ScrollDown,
            column: 4,
            row: 2,
            modifiers: 0,
        };
        assert!(
            ClientRequest::Pointer {
                event: wheel,
                frame: FrameId::from_raw(1),
                repetitions: MAX_POINTER_REPETITIONS,
            }
            .validate()
            .is_ok()
        );
        for repetitions in [0, MAX_POINTER_REPETITIONS + 1] {
            assert!(
                ClientRequest::Pointer {
                    event: wheel,
                    frame: FrameId::from_raw(1),
                    repetitions,
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            ClientRequest::Pointer {
                event: input::PointerEvent {
                    kind: input::PointerEventKind::Down(input::PointerButton::Left),
                    ..wheel
                },
                frame: FrameId::from_raw(1),
                repetitions: 2,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn request_decoding_rejects_unknown_fields() {
        let top_level = r#"{"type":"resize","geometry":{"screen":{"x":0,"y":0,"width":80,"height":24},"editor":{"x":0,"y":0,"width":80,"height":22},"global_status_line":{"x":0,"y":22,"width":80,"height":1},"interaction_line":{"x":0,"y":23,"width":80,"height":1}},"force":true}"#;
        assert!(serde_json::from_str::<ClientRequest>(top_level).is_err());

        let nested = r#"{"type":"resize","geometry":{"screen":{"x":0,"y":0,"width":80,"height":24,"depth":8},"editor":{"x":0,"y":0,"width":80,"height":22},"global_status_line":{"x":0,"y":22,"width":80,"height":1},"interaction_line":{"x":0,"y":23,"width":80,"height":1}}}"#;
        assert!(serde_json::from_str::<ClientRequest>(nested).is_err());
    }

    #[test]
    fn maximum_terminal_geometry_fits_the_wire_frame_budget() {
        let cell = frame::TerminalCell {
            character: '\u{10ffff}',
            combining: vec!['\u{10ffff}'; 3],
            width: 2,
            foreground: frame::TerminalColor::Rgb(255, 255, 255),
            background: frame::TerminalColor::Rgb(255, 255, 255),
            attributes: u16::MAX,
        };
        let columns = 256;
        let rows = input::FrameGeometry::MAX_CELLS / columns;
        let terminal = frame::TerminalView {
            revision: u64::MAX,
            columns,
            rows: vec![vec![cell; columns]; rows],
            line_ids: vec![Some(u64::MAX); rows],
            cursor: Some((rows - 1, columns - 1)),
            scrollback: usize::MAX,
            live: true,
            review: true,
            newer_output: true,
            highlights: Vec::new(),
        };
        let bytes = serde_json::to_vec(&terminal).unwrap();
        assert!(
            bytes.len() < 8 * 1024 * 1024 - 256 * 1024,
            "maximum terminal view encoded to {} bytes",
            bytes.len()
        );
    }

    #[test]
    fn long_running_action_snapshot_round_trips_over_the_wire() {
        let core = crate::snapshot::LongRunningActionSnapshot {
            label: "Indexing 👩‍💻".to_owned(),
            detail: "/workspace/界".to_owned(),
            elapsed_millis: 1_280,
            cancel_hint: Some(":stop-index".to_owned()),
        };
        let wire: frame::LongRunningActionSnapshot = core.clone().into();
        let encoded = serde_json::to_string(&wire).unwrap();
        let decoded: frame::LongRunningActionSnapshot = serde_json::from_str(&encoded).unwrap();
        let round_trip: crate::snapshot::LongRunningActionSnapshot = decoded.into();
        assert_eq!(round_trip, core);
    }

    #[test]
    fn notification_frame_values_and_ui_vocabulary_round_trip_over_v11() {
        let counts = crate::notification::NotificationCounts {
            errors: 3,
            warnings: 2,
            infos: 1,
        };
        let wire_counts: frame::NotificationCounts = counts.into();
        let encoded = serde_json::to_string(&wire_counts).unwrap();
        let decoded: frame::NotificationCounts = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            crate::notification::NotificationCounts::from(decoded),
            counts
        );

        for severity in [
            crate::notification::NotificationSeverity::Error,
            crate::notification::NotificationSeverity::Warning,
            crate::notification::NotificationSeverity::Info,
        ] {
            let wire: frame::NotificationSeverity = severity.into();
            let encoded = serde_json::to_string(&wire).unwrap();
            let decoded: frame::NotificationSeverity = serde_json::from_str(&encoded).unwrap();
            assert_eq!(
                crate::notification::NotificationSeverity::from(decoded),
                severity
            );
        }

        let (_, theme) = crate::config::Config::default().startup_theme().unwrap();
        let wire_theme: frame::Theme = theme.clone().into();
        let encoded = serde_json::to_string(&wire_theme).unwrap();
        let decoded: frame::Theme = serde_json::from_str(&encoded).unwrap();
        let round_trip: crate::config::Theme = decoded.into();
        assert_eq!(round_trip, theme);

        let geometry = crate::app::FrameGeometry {
            screen: crate::layout::Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
            editor: crate::layout::Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 22,
            },
            status: crate::layout::Rect {
                x: 0,
                y: 22,
                width: 80,
                height: 1,
            },
            message: crate::layout::Rect {
                x: 0,
                y: 23,
                width: 80,
                height: 1,
            },
        };
        let wire_geometry: FrameGeometry = geometry.into();
        let encoded = serde_json::to_string(&wire_geometry).unwrap();
        assert!(encoded.contains("global_status_line"), "{encoded}");
        assert!(encoded.contains("interaction_line"), "{encoded}");
        let decoded: FrameGeometry = serde_json::from_str(&encoded).unwrap();
        assert_eq!(crate::app::FrameGeometry::from(decoded), geometry);
    }

    #[test]
    fn fuzzy_grep_snippet_round_trips_match_positions_over_the_wire() {
        let core = crate::snapshot::OverlayPreview::Snippet {
            lines: vec!["context".to_owned(), "direct needle".to_owned()],
            start_row: 4,
            focus_row: 5,
            emphasis: vec![7, 8, 9, 10, 11, 12],
        };
        let wire: frame::OverlayPreview = core.clone().into();
        let encoded = serde_json::to_string(&wire).unwrap();
        let decoded: frame::OverlayPreview = serde_json::from_str(&encoded).unwrap();
        let round_trip: crate::snapshot::OverlayPreview = decoded.into();
        assert_eq!(round_trip, core);
    }

    #[test]
    fn matched_text_preview_round_trips_match_positions_over_the_wire() {
        let core = crate::snapshot::OverlayPreview::MatchedText {
            lines: vec![
                "Ada · 2026-08-16".to_owned(),
                String::new(),
                "Refresh workspace Git state".to_owned(),
            ],
            emphasis: vec![20, 21, 22, 23, 24, 25, 26, 27, 28],
        };
        let wire: frame::OverlayPreview = core.clone().into();
        let encoded = serde_json::to_string(&wire).unwrap();
        let decoded: frame::OverlayPreview = serde_json::from_str(&encoded).unwrap();
        let round_trip: crate::snapshot::OverlayPreview = decoded.into();
        assert_eq!(round_trip, core);
    }

    #[test]
    fn workspace_values_convert_without_becoming_serializable() {
        let core = CoreWaitStatus::Pending {
            buffers: vec![CoreBufferId::from_raw(7)],
            remaining: vec![CoreBufferId::from_raw(7)],
        };
        assert_eq!(
            WaitStatus::from(core),
            WaitStatus::Pending {
                buffers: vec![BufferId(7)],
                remaining: vec![BufferId(7)],
            }
        );
    }

    #[test]
    fn semantic_session_preview_round_trips_over_the_wire() {
        let core = CoreSessionPreview {
            layout_panes: 2,
            panes: vec![CoreSessionPreviewPane {
                active: true,
                title: "[file] src/app.rs".to_owned(),
                kind: CoreSessionPreviewPaneKind::Buffer {
                    dirty: true,
                    read_only: false,
                },
                start_line: Some(42),
                lines: vec!["fn preview()".to_owned()],
            }],
            omitted_panes: 1,
            other_resources: vec!["[terminal] tests".to_owned()],
            omitted_resources: 3,
        };

        let wire: SessionPreview = core.clone().into();
        let encoded = serde_json::to_string(&wire).unwrap();
        let decoded: SessionPreview = serde_json::from_str(&encoded).unwrap();
        let round_trip: CoreSessionPreview = decoded.into();

        assert_eq!(round_trip, core);
    }

    #[test]
    fn welcome_validation_rejects_feature_drift() {
        let response = HostResponse::Welcome {
            protocol: VERSION,
            pid: 1,
            features: vec![FeatureGroup::Control],
            host_version: CLIENT_VERSION.to_owned(),
        };
        assert!(validate_welcome(&response, false).is_err());
    }
}
