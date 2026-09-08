// SPDX-License-Identifier: MPL-2.0

use super::{App, FsConfirmation, Mode};
use crate::{fs_plan::FsPlan, plugin::filesystem::Finished};

impl App {
    pub(crate) fn plugin_has_input_surface(&self) -> bool {
        self.mode == Mode::Command
            || self.has_input_overlay()
            || self.macro_replay.is_some()
            || self.recording_macro.is_some()
    }

    pub(crate) fn present_plugin_filesystem(&mut self, owner: usize, handle: String, plan: FsPlan) {
        self.confirmation_revision = self.confirmation_revision.wrapping_add(1);
        let buffer = self.active().buffer;
        self.plugins.filesystem_confirmation =
            Some((owner, handle, self.confirmation_revision, buffer));
        self.fs_confirmation = Some(FsConfirmation {
            buffer,
            plan,
            selected: 0,
        });
        self.plugins.presentation_dirty = true;
        self.status("Review application filesystem changes before applying");
    }

    pub(crate) fn cancel_plugin_filesystem(&mut self, owner: usize, handle: Option<&str>) {
        if self
            .plugins
            .filesystem_confirmation
            .as_ref()
            .is_some_and(|(id, plan, _, _)| {
                *id == owner && handle.is_none_or(|handle| handle == plan)
            })
        {
            let (_, plan, revision, _) = self.plugins.filesystem_confirmation.take().unwrap();
            if self.confirmation_revision == revision {
                self.fs_confirmation = None;
            }
            self.plugins.filesystem_finished.push((
                owner,
                Finished {
                    plan,
                    state: "cancelled",
                    applied: 0,
                    recovery: false,
                },
            ));
            self.plugins.presentation_dirty = true;
        }
    }

    pub(crate) fn sync_plugin_filesystem_confirmation(&mut self) {
        if let Some((owner, _, revision, buffer)) = self.plugins.filesystem_confirmation.as_ref()
            && (self.fs_confirmation.is_none()
                || *revision != self.confirmation_revision
                || !self.plugins.frontend_attached
                || self.host_buffer_is_closed(*buffer))
        {
            self.cancel_plugin_filesystem(*owner, None);
        }
    }

    pub(super) fn finish_plugin_filesystem(
        &mut self,
        result: Option<(usize, String, u64, usize)>,
        state: &'static str,
        applied: usize,
        recovery: bool,
    ) {
        if let Some((owner, plan, _, _)) = result {
            self.plugins.filesystem_finished.push((
                owner,
                Finished {
                    plan,
                    state,
                    applied,
                    recovery,
                },
            ));
        }
    }
}
