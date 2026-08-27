// SPDX-License-Identifier: MPL-2.0

//! In-process workspace ownership and shared service lifecycle values.
//!
//! Standalone and attached deployments use the same [`WorkspaceHost`]. The
//! former calls it directly; a later transport adapter will exchange bounded
//! DTOs without exposing `App` or service implementation types.

mod buffers;
#[cfg(unix)]
mod catalog;
mod host;
mod identity;
#[cfg(unix)]
pub mod lifecycle;
mod service;
#[cfg(unix)]
pub mod transport;

pub use buffers::{
    BufferContents, BufferId, BufferMetadata, BufferRevision, WaitStatus, WaitToken,
};
#[cfg(unix)]
pub use catalog::{
    ABBREVIATED_WORKSPACE_ID, MAX_WORKSPACE_NUMBER, RecordedWorkspace, WorkspaceEvent,
    WorkspaceRow, WorkspaceService, WorkspaceServiceHandle, abbreviated_id_width,
    clear_stopped_sessions, ensure_recent_workspace, known_workspaces, record_recent_workspace,
    record_workspace_activity, recorded_workspace_number, rename_known_workspace,
    resolve_known_workspace, resolve_known_workspace_from_directory,
};
pub use host::{
    BufferRequestError, FrameId, HostCommand, HostEvent, HostFrame, HostInputOutcome,
    HostServiceSubmitError, SessionPreview, SessionPreviewPane, SessionPreviewPaneKind,
    WorkspaceHost,
};
pub use identity::WorkspaceIdentity;
pub use service::{
    CancellationToken, ServiceKind, ServiceLane, ServiceLifecycle, ServiceOutcome, ServicePhase,
    ServiceProgress, ServiceRequestId, ServiceStateError, ServiceSubmitError, ServiceUpdate,
    ServiceWorker,
};
