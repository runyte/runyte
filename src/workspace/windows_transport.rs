// SPDX-License-Identifier: MPL-2.0

//! Native adapters for the private bundled workspace protocol.
//! Lifecycle discovery and frontend availability are enabled separately.

mod buffered;
pub use buffered::BufferedLocalClient;
mod client;
mod server;
pub use super::transport_shared::{ResponseReceiver, ResponseSender, response_channel};
pub use crate::protocol::{
    CLIENT_VERSION, ClientKind, ClientRequest, ClientRole, FeatureGroup, HostResponse,
    MAX_FEATURE_GROUPS, TransportChange, decode_path, encode_path,
};
pub use client::LocalClient;
pub use server::LocalServer;
pub type ServerEvent = super::transport_shared::ServerEvent<
    std::sync::Arc<super::windows_process_identity::PinnedProcess>,
>;
pub const PROTOCOL_VERSION: u32 = crate::protocol::VERSION;
