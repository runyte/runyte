// SPDX-License-Identifier: MPL-2.0

//! In-process workspace ownership and shared service lifecycle values.
//!
//! Standalone and attached deployments use the same [`WorkspaceHost`]. The
//! former calls it directly; a later transport adapter will exchange bounded
//! DTOs without exposing `App` or service implementation types.

mod buffers;
#[cfg(unix)]
mod catalog;
pub mod context;
mod host;
mod identity;
#[cfg(unix)]
pub mod lifecycle;
#[cfg(unix)]
pub mod parent;
mod service;
#[cfg(unix)]
pub mod transport;
// Compile the shared boundary natively before enabling the Windows adapter.
#[cfg(any(unix, windows))]
#[cfg_attr(windows, allow(dead_code))] // Native adapter wiring follows separately.
mod transport_shared;
#[cfg(windows)]
pub mod windows_endpoint;
#[cfg(windows)]
pub mod windows_pipe;
#[cfg(windows)]
pub mod windows_process_identity;

pub use buffers::{
    BufferContents, BufferId, BufferMetadata, BufferRevision, WaitStatus, WaitToken,
};
#[cfg(unix)]
pub use catalog::{
    ABBREVIATED_WORKSPACE_ID, DestinationInventory, MAX_WORKSPACE_NUMBER, RecordedWorkspace,
    WorkspaceEvent, WorkspaceRow, WorkspaceService, WorkspaceServiceHandle, abbreviated_id_width,
    clear_stopped_sessions, ensure_recent_workspace, known_workspaces,
    known_workspaces_all_namespaces, known_workspaces_for_navigation, record_recent_workspace,
    record_workspace_activity, recorded_workspace_number, rename_known_workspace,
    resolve_known_workspace, resolve_known_workspace_from_directory,
};
pub use host::{
    BufferRequestError, FrameId, HostCommand, HostEvent, HostFrame, HostInputOutcome,
    HostServiceSubmitError, SessionPreview, SessionPreviewPane, SessionPreviewPaneKind,
    TERMINAL_OUTPUT_QUIET_INTERVAL, WorkspaceHost,
};
pub use identity::{WORKSPACE_ID_LENGTH, WorkspaceIdentity, workspace_id};
pub use service::{
    CancellationToken, ServiceKind, ServiceLane, ServiceLifecycle, ServiceOutcome, ServicePhase,
    ServiceProgress, ServiceRequestId, ServiceStateError, ServiceSubmitError, ServiceUpdate,
    ServiceWorker,
};
