// SPDX-License-Identifier: MPL-2.0

//! Receipt-driven completion of native session switches.

use super::{BufferedLocalClient, ClientRequest, FINAL_REPLY_BUDGET, HostResponse};
use anyhow::{Context, Result, bail};
use tokio::time::{Instant, timeout_at};

// Keep the exchange independent of process-exit notification: EOF and queued
// replies belong to the buffered reader, even after the host process exits.
trait Transport {
    async fn send(&mut self, request: &ClientRequest) -> Result<()>;
    async fn recv(&mut self) -> Result<Option<HostResponse>>;
}

impl Transport for BufferedLocalClient {
    async fn send(&mut self, request: &ClientRequest) -> Result<()> {
        self.send(request).await
    }

    async fn recv(&mut self) -> Result<Option<HostResponse>> {
        self.recv().await
    }
}

pub(super) async fn complete(
    client: &mut BufferedLocalClient,
    receipt: u64,
    committed: bool,
    deadline: Instant,
) -> Result<()> {
    exchange(client, receipt, committed, deadline).await
}

async fn exchange(
    client: &mut impl Transport,
    receipt: u64,
    committed: bool,
    operation_deadline: Instant,
) -> Result<()> {
    let request = if committed {
        ClientRequest::NativeSwitchCommit { receipt }
    } else {
        ClientRequest::NativeSwitchAbort { receipt }
    };
    // A host can apply the request, queue its receipt and close the pipe before
    // the local writer observes flush completion. A write error does not prove
    // that the switch failed; only the matching semantic receipt proves success.
    let mut send_error = timeout_at(operation_deadline, client.send(&request))
        .await
        .context("native switch send exceeded its whole-operation deadline")?
        .err();
    let deadline = operation_deadline.min(Instant::now() + FINAL_REPLY_BUDGET);
    let result = async {
        let mut confirming_parent = false;
        loop {
            let response = timeout_at(deadline, client.recv())
                .await
                .context("source workspace host did not acknowledge native switch")??
                .context("source workspace host disconnected before switch acknowledgement")?;
            match response {
                HostResponse::NativeSwitchCommitted { receipt: received }
                    if committed && received == receipt =>
                {
                    return Ok(());
                }
                HostResponse::NativeSwitchAborted { receipt: received }
                    if !committed && received == receipt =>
                {
                    return Ok(());
                }
                HostResponse::NativeParentSwitchCommitAccepted { receipt: received }
                    if committed && received == receipt && !confirming_parent =>
                {
                    let confirmation = timeout_at(
                        deadline,
                        client.send(&ClientRequest::NativeParentSwitchCommitObserved { receipt }),
                    )
                    .await
                    .context("source frontend did not confirm parent switch commit")?;
                    match confirmation {
                        // The source's acceptance was observed and confirmed on
                        // this connection. Losing its final receipt is permitted
                        // after this irreversible handoff, as before.
                        Ok(()) => return Ok(()),
                        Err(error) => {
                            send_error.get_or_insert(error);
                            confirming_parent = true;
                            // Acceptance alone cannot prove the failed send was
                            // applied. Require the final matching commit receipt.
                        }
                    }
                }
                HostResponse::WaitState { .. }
                | HostResponse::Frame { .. }
                | HostResponse::TerminalDamage { .. }
                | HostResponse::EditorDamage { .. } => {}
                HostResponse::Error { message } | HostResponse::Refused { message } => {
                    bail!(message)
                }
                response => bail!("unexpected native switch acknowledgement: {response:?}"),
            }
        }
    }
    .await;
    match (result, send_error) {
        (Err(error), Some(send_error)) => Err(send_error.context(format!(
            "native switch send failed and no matching acknowledgement arrived: {error:#}"
        ))),
        (result, _) => result,
    }
}

#[cfg(test)]
#[path = "switch/tests.rs"]
mod tests;
