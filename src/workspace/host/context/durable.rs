// SPDX-License-Identifier: MPL-2.0

//! Blocking storage belongs to one bounded worker, never to the editor loop.
use super::*;
use crate::workspace::context::{
    durable::{ConversationKind, ReadMode, Route, SendKey, Store},
    durable_protocol::{Command, Operation},
};
use std::{
    path::PathBuf,
    sync::mpsc::{SyncSender, sync_channel},
};
use tokio::sync::oneshot;

pub(super) struct Worker(SyncSender<Job>);
struct Job {
    command: Command,
    route: Route,
    registration: Registration,
    id: String,
    lease: Arc<Lease>,
    reply: oneshot::Sender<Reply>,
}
impl Worker {
    #[cfg(all(test, unix))]
    pub(super) fn disconnected_for_test() -> Self {
        Self(sync_channel(0).0)
    }
    fn start(root: PathBuf) -> std::io::Result<Self> {
        // Each admitted command is bounded to < 40 KiB by the typed wire.
        // Only one result is retained by this worker; transport reply queues
        // retain their existing per-connection limits and cancellation leases.
        let (send, receive) = sync_channel::<Job>(16);
        std::thread::Builder::new().name("runyte-channel".into()).spawn(move || {
            let mut store = None;
            while let Ok(job) = receive.recv() {
                if !job.lease.active() || job.reply.is_closed() { continue; }
                let result = (|| -> anyhow::Result<Value> {
                    if store.is_none() { store = Some(Store::open(&root)?); }
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis().try_into()?;
                    execute(store.as_mut().unwrap(), &job.route, &job.command, now)
                })();
                // Revocation cancels queued work and hides replies. A transaction
                // admitted before cancellation may commit; retry its original key
                // after authorization is restored to resolve uncertain outcomes.
                if !job.lease.active() || job.reply.is_closed() { continue; }
                let value = match result {
                    Ok(mut result) => {
                        result["workspace"] = json!(job.registration);
                        json!({"type":"response","id":job.id,"result":result})
                    }
                    // Do not leak SQL, paths, credentials or database contents.
                    Err(_) => json!({"type":"response","id":job.id,"error":Error::new(Code::Unavailable,"Durable operation refused; check membership, receipt, retry key, limits and storage availability")}),
                };
                let _ = job.reply.send(Reply { value, lease:Some(job.lease), close:false });
            }
        })?;
        Ok(Self(send))
    }
}
fn execute(store: &mut Store, route: &Route, command: &Command, now: u64) -> anyhow::Result<Value> {
    if let Operation::Register { provider, task } = &command.operation {
        let actor = store.register(route, &command.credential.0, provider, task)?;
        return Ok(json!({"participant":actor.id()}));
    }
    let actor = store.resume(route, &command.credential.0)?;
    Ok(match &command.operation {
        Operation::Resume {} => json!({"participant":actor.id()}),
        Operation::Create {
            name,
            fixed,
            members,
        } => {
            json!({"channel":store.create(&actor,name,if *fixed {ConversationKind::Fixed} else {ConversationKind::Public},members)?})
        }
        Operation::Subscribe { channel } => {
            store.subscribe(&actor, channel)?;
            json!({})
        }
        Operation::Post {
            channel,
            nonce,
            issued_ms,
            text,
            location,
        } => {
            json!({"seq":store.post(&actor,channel,&SendKey {nonce:nonce.clone(),issued_ms:*issued_ms},text,location,now)?})
        }
        Operation::Read { channel, limit } => {
            serde_json::to_value(store.read(&actor, channel, ReadMode::Unread, *limit, now)?)?
        }
        Operation::History {
            channel,
            limit,
            before,
        } => serde_json::to_value(store.read(
            &actor,
            channel,
            ReadMode::History { before: *before },
            *limit,
            now,
        )?)?,
        Operation::Acknowledge { channel, receipt } => {
            store.acknowledge(&actor, channel, receipt, now)?;
            json!({})
        }
        Operation::Register { .. } => unreachable!(),
    })
}
impl WorkspaceHost {
    pub(super) fn context_durable_frame(
        &mut self,
        connection: u64,
        bytes: &[u8],
        reply: &mut Option<oneshot::Sender<Reply>>,
    ) -> bool {
        let Ok(envelope) = std::str::from_utf8(bytes)
            .map_err(|_| ())
            .and_then(|raw| wire::parse_request(raw).map_err(|_| ()))
        else {
            return false;
        };
        let Request::ChannelDurable(command) = envelope.request else {
            return false;
        };
        let Some(reader) = self
            .context
            .readers
            .get_mut(&connection)
            .filter(|r| r.registered)
        else {
            return false;
        };
        let result = (|| -> Result<(), Error> {
            if !reader.lease.active() || !reader.scopes.contains(&Scope::AgentChannel) {
                return Err(Error::new(
                    Code::CapabilityDenied,
                    "Channel scope not granted",
                ));
            }
            if !reader.durable {
                return Err(Error::new(
                    Code::Unsupported,
                    "Durable feature was not negotiated",
                ));
            }
            if reader.requests.len() >= wire::MAX_DEDUPLICATED_REQUESTS {
                return Err(Error::new(
                    Code::LimitExceeded,
                    "Reader request lifetime exhausted; reconnect",
                ));
            }
            if !reader.requests.insert(envelope.id.clone()) {
                return Err(Error::new(Code::Conflict, "Request already used"));
            }
            let root = self.context.durable_root.as_ref().ok_or_else(|| {
                Error::new(Code::Unavailable, "Durable storage is not configured")
            })?;
            if !root.is_absolute()
                || command.store != crate::hash::sha256_hex(&crate::native_path::encode_path(root))
            {
                return Err(Error::new(
                    Code::CapabilityDenied,
                    "Profile belongs to another durable store",
                ));
            }
            if self.context.durable_worker.is_none() {
                self.context.durable_worker =
                    Some(Worker::start(root.clone()).map_err(|_| {
                        Error::new(Code::Unavailable, "Cannot start channel worker")
                    })?);
            }
            Ok(())
        })();
        let registration = self.context.registration.clone();
        let error = match (result, registration) {
            (Ok(()), Some(registration)) => {
                let job = Job {
                    command,
                    route: Route {
                        workspace: registration.workspace_id.clone(),
                        identity: reader.fingerprint.clone(),
                    },
                    registration,
                    id: envelope.id.clone(),
                    lease: reader.lease.clone(),
                    reply: reply.take().unwrap(),
                };
                match self
                    .context
                    .durable_worker
                    .as_ref()
                    .unwrap()
                    .0
                    .try_send(job)
                {
                    Ok(()) => return true,
                    Err(error) => {
                        let job = match error {
                            std::sync::mpsc::TrySendError::Full(job)
                            | std::sync::mpsc::TrySendError::Disconnected(job) => job,
                        };
                        *reply = Some(job.reply);
                        Error::new(Code::Busy, "Durable worker queue unavailable; retry later")
                    }
                }
            }
            (Err(error), _) => error,
            _ => Error::new(Code::Unavailable, "Workspace registration unavailable"),
        };
        let _ = reply.take().unwrap().send(Reply {
            value: json!({"type":"response","id":envelope.id,"error":error}),
            lease: Some(reader.lease.clone()),
            close: false,
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_queued_job_is_skipped_and_live_job_can_follow_it() {
        let root = crate::test_support::TestRuntimeRoot::new("durable-live").unwrap();
        let db = root.path().join("db");
        let worker = Worker::start(db.clone()).unwrap();
        let route = Route {
            workspace: "fixture".into(),
            identity: "test".into(),
        };
        let registration = Registration {
            root: root.path().into(),
            workspace_id: "fixture".into(),
            host_incarnation: "fixture".into(),
            endpoint: root.path().join("endpoint"),
            mode: storage::HostMode::Standalone,
            pid: std::process::id(),
            environment: "fixture".into(),
            #[cfg(windows)]
            creation_time: 0,
        };
        let command = |secret| Command {
            credential: crate::workspace::context::durable_protocol::Credential(secret),
            store: "b".repeat(64),
            operation: Operation::Register {
                provider: "test".into(),
                task: "queue cancellation".into(),
            },
        };
        let cancelled = Arc::new(Lease::default());
        cancelled.cancel();
        let (send, receive) = oneshot::channel();
        assert!(
            worker
                .0
                .try_send(Job {
                    command: command("a".repeat(64)),
                    route: route.clone(),
                    registration: registration.clone(),
                    id: "cancelled".into(),
                    lease: cancelled,
                    reply: send
                })
                .is_ok()
        );
        let (send, live) = oneshot::channel();
        assert!(
            worker
                .0
                .try_send(Job {
                    command: command("c".repeat(64)),
                    route: route.clone(),
                    registration,
                    id: "live".into(),
                    lease: Arc::new(Lease::default()),
                    reply: send
                })
                .is_ok()
        );
        let result = tokio::time::timeout(Duration::from_secs(3), live)
            .await
            .unwrap()
            .unwrap();
        assert!(result.value["result"]["participant"].is_string());
        assert!(receive.await.is_err());
        let store = Store::open(&db).unwrap();
        assert!(store.resume(&route, &"a".repeat(64)).is_err());
        assert!(store.resume(&route, &"c".repeat(64)).is_ok());
    }
}
