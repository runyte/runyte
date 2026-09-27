// SPDX-License-Identifier: MPL-2.0

use super::*;
use anyhow::anyhow;
use std::{collections::VecDeque, time::Duration};

#[derive(Default)]
struct Peer {
    sends: VecDeque<Result<()>>,
    replies: VecDeque<HostResponse>,
    requests: Vec<ClientRequest>,
    silent: bool,
}

impl Transport for Peer {
    async fn send(&mut self, request: &ClientRequest) -> Result<()> {
        self.requests.push(request.clone());
        self.sends.pop_front().expect("unexpected request")
    }

    async fn recv(&mut self) -> Result<Option<HostResponse>> {
        if self.silent && self.replies.is_empty() {
            std::future::pending().await
        } else {
            Ok(self.replies.pop_front())
        }
    }
}

fn lost_ack() -> Result<()> {
    Err(anyhow!(
        "native buffered transport write acknowledgement lost"
    ))
}

async fn run(peer: &mut Peer, committed: bool) -> Result<()> {
    exchange(peer, 9, committed, Instant::now() + Duration::from_secs(1)).await
}

fn wait_state() -> HostResponse {
    HostResponse::WaitState {
        token: serde_json::from_str("1").unwrap(),
        status: runyte::protocol::WaitStatus::Pending {
            buffers: Vec::new(),
            remaining: Vec::new(),
        },
        interactive_attached: true,
    }
}

#[tokio::test]
async fn matching_receipt_recovers_lost_write_ack_before_eof() {
    for committed in [false, true] {
        for failed_send in [false, true] {
            let mut peer = Peer {
                sends: VecDeque::from([if failed_send { lost_ack() } else { Ok(()) }]),
                replies: VecDeque::from([
                    wait_state(),
                    if committed {
                        HostResponse::NativeSwitchCommitted { receipt: 9 }
                    } else {
                        HostResponse::NativeSwitchAborted { receipt: 9 }
                    },
                ]),
                ..Peer::default()
            };
            run(&mut peer, committed).await.unwrap();
            assert_eq!(peer.requests.len(), 1);
            assert!(matches!(
                (&peer.requests[0], committed),
                (ClientRequest::NativeSwitchCommit { receipt: 9 }, true)
                    | (ClientRequest::NativeSwitchAbort { receipt: 9 }, false)
            ));
        }
    }
}

#[tokio::test]
async fn lost_write_ack_does_not_turn_missing_stale_or_unrelated_replies_into_success() {
    for committed in [false, true] {
        for reply in [
            None,
            Some(HostResponse::NativeSwitchCommitted { receipt: 8 }),
            Some(HostResponse::NativeSwitchAborted { receipt: 8 }),
            Some(if committed {
                HostResponse::NativeSwitchAborted { receipt: 9 }
            } else {
                HostResponse::NativeSwitchCommitted { receipt: 9 }
            }),
            Some(HostResponse::Detached {
                directory_bytes: None,
            }),
            Some(HostResponse::ShuttingDown),
            Some(HostResponse::Error {
                message: "source failure".into(),
            }),
            Some(HostResponse::Refused {
                message: "source refusal".into(),
            }),
        ] {
            let mut peer = Peer {
                sends: VecDeque::from([lost_ack()]),
                replies: reply.into_iter().collect(),
                ..Peer::default()
            };
            let error = run(&mut peer, committed).await.unwrap_err();
            assert!(error.to_string().contains("no matching acknowledgement"));
            assert!(
                error
                    .root_cause()
                    .to_string()
                    .contains("write acknowledgement lost")
            );
        }
    }
}

#[tokio::test]
async fn parent_acceptance_requires_confirmation_or_matching_final_receipt() {
    for failed_confirmation in [false, true] {
        for final_receipt in [None, Some(8), Some(9)] {
            let mut peer = Peer {
                sends: VecDeque::from([
                    Ok(()),
                    if failed_confirmation {
                        lost_ack()
                    } else {
                        Ok(())
                    },
                ]),
                replies: VecDeque::from([HostResponse::NativeParentSwitchCommitAccepted {
                    receipt: 9,
                }]),
                ..Peer::default()
            };
            if let Some(receipt) = final_receipt {
                peer.replies
                    .push_back(HostResponse::NativeSwitchCommitted { receipt });
            }
            let result = run(&mut peer, true).await;
            assert_eq!(
                result.is_ok(),
                !failed_confirmation || final_receipt == Some(9)
            );
            assert!(matches!(
                peer.requests.as_slice(),
                [
                    ClientRequest::NativeSwitchCommit { receipt: 9 },
                    ClientRequest::NativeParentSwitchCommitObserved { receipt: 9 },
                ]
            ));
        }
    }
    let mut peer = Peer {
        sends: VecDeque::from([lost_ack(), lost_ack()]),
        replies: VecDeque::from([
            HostResponse::NativeParentSwitchCommitAccepted { receipt: 9 },
            HostResponse::NativeParentSwitchCommitAccepted { receipt: 9 },
        ]),
        ..Peer::default()
    };
    assert!(run(&mut peer, true).await.is_err());
    assert_eq!(peer.requests.len(), 2);
}

#[tokio::test]
async fn silent_source_recovery_keeps_the_operation_deadline() {
    let mut peer = Peer {
        sends: VecDeque::from([lost_ack()]),
        silent: true,
        ..Peer::default()
    };
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        exchange(
            &mut peer,
            9,
            true,
            Instant::now() + Duration::from_millis(10),
        ),
    )
    .await
    .expect("switch recovery must not wait past its operation deadline")
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("did not acknowledge native switch")
    );
    assert!(
        error
            .root_cause()
            .to_string()
            .contains("write acknowledgement lost")
    );
}
