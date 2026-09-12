// SPDX-License-Identifier: MPL-2.0

use super::App;
use anyhow::{Result, ensure};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub(crate) struct State {
    pub request: Option<Request>,
    pub cancellation: Option<Arc<AtomicBool>>,
}

pub(crate) struct Request {
    pub buffer: usize,
    pub revision: u64,
    pub spans: Vec<(usize, usize)>,
    pub command: String,
    pub inputs: Vec<String>,
    pub directory: std::path::PathBuf,
}

impl App {
    pub(super) fn request_pipe(&mut self, command: String) -> Result<()> {
        ensure!(cfg!(unix), "shell pipes require Unix");
        ensure!(
            self.pipe.cancellation.is_none(),
            "a pipe is already running; use :pipe-cancel"
        );
        ensure!(command.len() <= 16 * 1024, "pipe command exceeds 16 KiB");
        ensure!(
            self.active_terminal().is_none(),
            "pipe requires an editable buffer"
        );
        let buffer = self.active().buffer;
        ensure!(
            !self.buffers[buffer].is_read_only(),
            "pipe buffer is read-only"
        );
        let spans = self.operative_spans();
        ensure!(spans.len() <= 256, "pipe supports at most 256 selections");
        ensure!(
            spans.windows(2).all(|s| s[0].1 <= s[1].0),
            "pipe selection spans overlap"
        );
        ensure!(
            spans.iter().map(|(from, to)| to - from).sum::<usize>() <= crate::pipe::MAX_BYTES,
            "pipe input exceeds 8 MiB"
        );
        let mut inputs = Vec::new();
        let mut bytes = 0;
        for &(from, to) in &spans {
            let text = self.buffers[buffer].slice(from, to);
            bytes += text.len();
            ensure!(bytes <= crate::pipe::MAX_BYTES, "pipe input exceeds 8 MiB");
            inputs.push(text);
        }
        self.pipe.request = Some(Request {
            buffer,
            revision: self.buffers[buffer].revision(),
            spans,
            command,
            inputs,
            directory: self.project_root.clone(),
        });
        self.pipe.cancellation = Some(Arc::new(AtomicBool::new(false)));
        self.status("Pipe started; :pipe-cancel cancels");
        Ok(())
    }

    pub(crate) fn cancel_pipe(&mut self) {
        if let Some(cancel) = &self.pipe.cancellation {
            cancel.store(true, Ordering::Release);
        }
    }

    pub(crate) fn pipe_finished(&mut self, result: Result<(), String>, protective: bool) {
        self.pipe.cancellation = None;
        match result {
            Ok(()) => self.status("Pipe completed"),
            Err(message) if protective => self.action_warning("Pipe result refused", message),
            Err(message) => self.error_from("Pipe", "Pipe failed", message),
        }
    }
}
