// SPDX-License-Identifier: MPL-2.0

//! In-process workspace ownership and shared service lifecycle values.
//!
//! Standalone and attached deployments use the same [`WorkspaceHost`]. The
//! former calls it directly; a later transport adapter will exchange bounded
//! DTOs without exposing `App` or service implementation types.

mod buffers;
#[cfg(unix)]
mod catalog;
#[cfg(any(unix, windows))]
#[cfg_attr(windows, allow(dead_code))] // Native catalog callers follow host acceptance.
mod catalog_values;
pub mod context;
mod host;
mod identity;
#[cfg(unix)]
pub mod lifecycle;
#[cfg(unix)]
pub mod parent;
#[cfg(any(unix, windows))]
#[cfg_attr(windows, allow(dead_code))] // Catalog wiring follows native history acceptance.
mod recent_history;
mod service;
#[cfg(any(unix, windows))]
mod session_name;
#[cfg(unix)]
pub mod transport;
// Shared bounded framing and native/Unix adapter support.
#[cfg(any(unix, windows))]
mod transport_shared;
#[cfg(windows)]
pub mod windows_endpoint;
#[cfg(windows)]
pub mod windows_lifecycle;
#[cfg(windows)]
pub mod windows_location;
#[cfg(windows)]
pub mod windows_pipe;
#[cfg(windows)]
pub mod windows_process_identity;
#[cfg(windows)]
pub mod windows_startup;
#[cfg(windows)]
pub mod windows_transport;

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
