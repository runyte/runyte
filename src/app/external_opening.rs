// SPDX-License-Identifier: MPL-2.0

//! Captured native open requests and nonblocking dispatch completion.
use super::*;
use crate::external_open::{Dispatch, LaunchTicket};

const MAX_PENDING: usize = 16;

pub(super) enum Intent {
    Browser(String),
    Directory(PathBuf),
    Program { path: PathBuf, program: String },
}

pub(super) struct Pending {
    intent: Intent,
    ticket: LaunchTicket,
}

impl App {
    pub(super) fn dispatch_external_open(&mut self, intent: Intent) {
        if self.pending_external_opens.len() >= MAX_PENDING {
            self.action_failed("External opening request limit reached");
            return;
        }
        let result = match &intent {
            Intent::Browser(url) => (self.ports.browser)(url),
            Intent::Directory(path) => (self.ports.directory_opener)(path),
            Intent::Program { path, program } => (self.ports.program_opener)(program, path),
        };
        match result {
            Ok(Dispatch::Accepted) => self.finish_external_open(intent, Ok(())),
            Ok(Dispatch::Pending(ticket)) => {
                self.pending_external_opens.push(Pending { intent, ticket });
                self.status("Opening external application…");
            }
            Err(error) => self.finish_external_open(intent, Err(error)),
        }
    }

    /// Drain only ready completions during the existing host maintenance tick.
    /// A dropped workspace drops its tickets; dispatch already handed to the
    /// operating system is neither retried nor revoked by an editor transition.
    pub fn poll_external_opens(&mut self, now: Instant) -> bool {
        let mut changed = false;
        let mut index = 0;
        while index < self.pending_external_opens.len() {
            if let Some(result) = self.pending_external_opens[index].ticket.poll(now) {
                let pending = self.pending_external_opens.remove(index);
                self.finish_external_open(pending.intent, result);
                changed = true;
            } else {
                index += 1;
            }
        }
        changed
    }

    fn finish_external_open(&mut self, intent: Intent, result: Result<()>) {
        if let Err(error) = result {
            let (source, title) = match intent {
                Intent::Browser(_) => ("Browser", "Browser launch failed"),
                Intent::Directory(_) => ("File manager", "System file manager launch failed"),
                Intent::Program { .. } => ("External program", "Program launch failed"),
            };
            self.error_from(source, title, error.to_string());
            return;
        }
        match intent {
            Intent::Browser(url) => self.status(format!("opened {url} in the default browser")),
            Intent::Directory(path) => self.status(format!(
                "opened {} in the system file manager",
                path.display()
            )),
            Intent::Program { path, program } => {
                if program.is_empty() {
                    self.status(format!(
                        "opened {} with the system default application",
                        path.display()
                    ));
                } else if let Err(error) = self.programs.remember(&program) {
                    self.action_warning(
                        "Program choice was not saved",
                        format!("opened with {program}, but {error}"),
                    );
                } else {
                    self.status(format!("opened {} with {program}", path.display()));
                }
            }
        }
    }
}
