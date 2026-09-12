// SPDX-License-Identifier: MPL-2.0

use super::{BufferId, BufferRevision, WorkspaceHost};
use crate::{
    app::pipe::Request,
    pipe::Completion,
    text::{Change, Transaction},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Worker {
    request: Request,
    cancel: Arc<AtomicBool>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl WorkspaceHost {
    /// Called once when host services start; attachment never creates workers.
    pub fn start_pipe_service(&mut self) -> tokio::sync::mpsc::Receiver<Completion> {
        assert!(self.pipe_events.is_none(), "pipe service starts once");
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        self.pipe_events = Some(sender);
        receiver
    }

    pub(super) fn sync_pipe(&mut self) {
        if let Some(worker) = &self.pipe_worker {
            let r = &worker.request;
            if self.app.host_buffer_is_closed(r.buffer)
                || self.app.buffers[r.buffer].revision() != r.revision
                || self.app.buffers[r.buffer].is_read_only()
            {
                worker.cancel.store(true, Ordering::Release);
            }
            return;
        }
        let Some(mut request) = self.app.pipe.request.take() else {
            return;
        };
        let Some(sender) = self.pipe_events.clone() else {
            self.app
                .pipe_finished(Err("pipe service is unavailable".into()), false);
            return;
        };
        let cancel = self.app.pipe.cancellation.as_ref().unwrap().clone();
        let worker_cancel = cancel.clone();
        let command = request.command.clone();
        let directory = request.directory.clone();
        let inputs = std::mem::take(&mut request.inputs);
        let task = tokio::task::spawn_blocking(move || {
            let completion = crate::pipe::run(&command, &directory, inputs, worker_cancel);
            let _ = sender.blocking_send(completion);
        });
        self.pipe_worker = Some(Worker {
            request,
            cancel,
            task: Some(task),
        });
    }

    pub fn handle_pipe_completion(&mut self, completion: Completion) {
        let Some(worker) = self.pipe_worker.take() else {
            return;
        };
        let r = &worker.request;
        let refusal = if self.app.host_buffer_is_closed(r.buffer) {
            Some("invoking buffer closed")
        } else if self.app.buffers[r.buffer].revision() != r.revision {
            Some("invoking buffer changed")
        } else if self.app.buffers[r.buffer].is_read_only() {
            Some("invoking buffer is read-only")
        } else if worker.cancel.load(Ordering::Acquire) {
            Some("pipe cancelled")
        } else {
            None
        };
        if let Some(reason) = refusal {
            self.app.pipe_finished(Err(reason.into()), true);
            return;
        }
        let result = completion.0.and_then(|outputs| {
            let transaction = Transaction::new(
                r.spans
                    .iter()
                    .zip(outputs)
                    .map(|(&(from, to), text)| Change::new(from, to, text))
                    .collect(),
            );
            if transaction.is_empty() {
                return Ok(());
            }
            self.app.buffers[r.buffer].commit_undo_group();
            self.apply_expected_transaction(
                BufferId::from_index(r.buffer),
                BufferRevision::from_raw(r.revision),
                transaction,
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        });
        self.app.pipe_finished(result, false);
    }

    pub(super) async fn shutdown_pipe(&mut self) {
        self.app.cancel_pipe();
        self.app.pipe.request = None;
        if let Some(mut worker) = self.pipe_worker.take() {
            worker.cancel.store(true, Ordering::Release);
            if let Some(task) = worker.task.take() {
                let _ = task.await;
            }
        }
    }
}

#[cfg(all(test, unix))]
#[path = "tests/pipe.rs"]
mod tests;
