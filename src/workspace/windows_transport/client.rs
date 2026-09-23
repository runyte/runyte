// SPDX-License-Identifier: MPL-2.0

use super::{
    CLIENT_VERSION, ClientKind, ClientRequest, ClientRole, FeatureGroup, HostResponse,
    PROTOCOL_VERSION,
};
use crate::{
    app::FrameGeometry,
    workspace::{
        transport_shared::{MessageReader, write_message},
        windows_endpoint::EndpointMetadata,
        windows_pipe::{self, Connection},
        windows_process_identity::PinnedProcess,
    },
};
use anyhow::{Context, Result, ensure};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncWrite, ReadHalf, WriteHalf},
    net::windows::named_pipe::NamedPipeClient,
    time::Instant,
};

type ClientPipe = Connection<NamedPipeClient>;
const CONNECT_BUDGET: Duration = Duration::from_secs(2);

/// An authenticated native connection for bundled control clients. The reader
/// remains available after an outgoing failure for bounded lifecycle recovery.
pub struct LocalClient {
    reader: MessageReader<ReadHalf<ClientPipe>>,
    writer: Option<WriteHalf<ClientPipe>>,
    peer: Arc<PinnedProcess>,
}

impl LocalClient {
    pub async fn connect(
        metadata: &EndpointMetadata,
        geometry: FrameGeometry,
        interactive: bool,
    ) -> Result<Self> {
        Self::connect_with_handoff(metadata, geometry, interactive, false).await
    }

    /// Metadata must come from the private endpoint boundary. Native connection
    /// admission independently checks the actual server identity before Hello.
    pub async fn connect_with_handoff(
        metadata: &EndpointMetadata,
        geometry: FrameGeometry,
        interactive: bool,
        directory_handoff: bool,
    ) -> Result<Self> {
        metadata.validate()?;
        ensure!(
            metadata.protocol == PROTOCOL_VERSION,
            "workspace host protocol {} is incompatible with client protocol {}",
            metadata.protocol,
            PROTOCOL_VERSION
        );
        let stream = windows_pipe::connect(metadata, Instant::now() + CONNECT_BUDGET).await?;
        let peer = stream.peer().clone();
        let (reader, writer) = tokio::io::split(stream);
        let mut client = Self {
            reader: MessageReader::new(reader),
            writer: Some(writer),
            peer,
        };
        client
            .send(&client_hello(
                metadata,
                geometry,
                interactive,
                directory_handoff,
            ))
            .await?;
        Ok(client)
    }

    pub fn peer(&self) -> &Arc<PinnedProcess> {
        &self.peer
    }

    /// A cancelled or failed send permanently closes the outgoing capability.
    /// Never retry an unknown partially written request on the same stream.
    pub async fn send(&mut self, request: &ClientRequest) -> Result<()> {
        send_request(&mut self.writer, request).await
    }

    /// Partial framing stays in the reader across cancellation by select!.
    pub async fn recv(&mut self) -> Result<Option<HostResponse>> {
        self.reader.read().await
    }
}

pub(super) fn client_hello(
    metadata: &EndpointMetadata,
    geometry: FrameGeometry,
    interactive: bool,
    directory_handoff: bool,
) -> ClientRequest {
    ClientRequest::Hello {
        protocol: PROTOCOL_VERSION,
        directory_handoff,
        features: if interactive {
            vec![
                FeatureGroup::Snapshots,
                FeatureGroup::Input,
                FeatureGroup::Buffers,
                FeatureGroup::Wait,
            ]
        } else {
            vec![
                FeatureGroup::Control,
                FeatureGroup::Buffers,
                FeatureGroup::Wait,
            ]
        },
        project_root_bytes: metadata.project_root_bytes.clone(),
        client_kind: if interactive {
            ClientKind::Tui
        } else {
            ClientKind::Control
        },
        client_version: CLIENT_VERSION.to_owned(),
        role: if interactive {
            ClientRole::Interactive
        } else {
            ClientRole::Control
        },
        geometry: geometry.into(),
    }
}

async fn send_request<W: AsyncWrite + Unpin>(
    writer: &mut Option<W>,
    request: &ClientRequest,
) -> Result<()> {
    let mut live = writer
        .take()
        .context("workspace transport writer is closed")?;
    // Native shutdown drains writes; invoking it after a stall could wait for
    // the same blocked peer indefinitely. Dropping only this split write half
    // retains incoming responses. Final client drop cancels the whole pipe.
    write_message(&mut live, request).await?;
    *writer = Some(live);
    Ok(())
}

#[cfg(test)]
mod tests;
