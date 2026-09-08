// SPDX-License-Identifier: MPL-2.0

//! Bounded model construction off the editor loop; installation is an optimistic commit.

use super::WorkspaceHost;
use crate::{
    buffer::PluginProjectionSource,
    plugin::{self, application as api, view},
};
use api::{Error, ErrorCode as Code, Request, ResultValue};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const INLINE_PREPARE: usize = 16 * 1024 * 1024;
const LARGE_PREPARE: usize = 32 * 1024 * 1024;
const HANDLE_CHARGE: usize = 1024;
const MAX_PREPARATIONS: usize = 2;

pub(super) fn is_model_request(request: &Request) -> bool {
    matches!(
        request,
        Request::ViewCreate { .. }
            | Request::ViewQuerySet { .. }
            | Request::ViewPublish { .. }
            | Request::ViewPatch { .. }
            | Request::ViewStageOpen { .. }
            | Request::ViewStageWrite { .. }
            | Request::ViewStageCommit { .. }
            | Request::ViewStageClose { .. }
            | Request::ViewSnapshotOpen { .. }
            | Request::ViewSnapshotRead { .. }
            | Request::ViewSnapshotClose { .. }
    )
}

enum Input {
    Model(view::Model),
    Staged(Arc<view::Model>, String),
    Patch(Arc<view::Model>, view::Patch),
    Encoded(Arc<view::Model>, view::StageKind, String),
}
impl Input {
    fn prepare(self, cancelled: &AtomicBool) -> Result<view::PreparedModel, Error> {
        if cancelled.load(Ordering::SeqCst) {
            return Err(Error::new(Code::Cancelled, "View preparation cancelled"));
        }
        let model = match self {
            Self::Staged(..) => unreachable!("stage admitted before spawning worker"),
            Self::Model(model) => model,
            Self::Patch(base, patch) => base.patched(patch, cancelled)?,
            Self::Encoded(base, kind, text) => match kind {
                view::StageKind::Model => serde_json::from_str(&text)
                    .map_err(|_| Error::new(Code::InvalidArgument, "Invalid staged view model"))?,
                view::StageKind::Patch => {
                    let patch = serde_json::from_str(&text).map_err(|_| {
                        Error::new(Code::InvalidArgument, "Invalid staged view patch")
                    })?;
                    base.patched(patch, cancelled)?
                }
            },
        };
        view::PreparedModel::build(model, cancelled)
    }
}

impl WorkspaceHost {
    pub(super) fn application_model_request(
        &mut self,
        owner: usize,
        request_id: &str,
        request: Request,
    ) -> Result<Option<ResultValue>, Error> {
        if !self.app.plugins.instances[&owner]
            .application
            .capabilities
            .contains("views")
        {
            return Err(Error::new(
                Code::CapabilityDenied,
                "Views capability was not granted",
            ));
        }
        match request {
            Request::ViewQuerySet {
                view: handle,
                expected_revision,
                expected_query_revision,
                text,
            } => {
                self.check_model_target(owner, &handle, &expected_revision)?;
                let live = &self.app.plugins.instances[&owner].application.views[&handle];
                let query = view::QueryState::set(
                    live.query.as_ref(),
                    expected_query_revision.as_deref(),
                    text,
                )?;
                let charge = if live.query.is_none() {
                    view::QUERY_CHARGE
                } else {
                    0
                };
                self.reserve_application_payload(owner, charge)?;
                let state = &mut self
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application;
                let live = state.views.get_mut(&handle).unwrap();
                live.charge += charge;
                live.query = Some(query);
                state.retained_payload += charge;
                self.app.refresh_plugin_view_title(owner, &handle);
                Ok(Some(ResultValue::ViewQuery {
                    view: handle.clone(),
                    revision: expected_revision,
                    query: self.app.plugins.instances[&owner].application.views[&handle]
                        .query
                        .as_ref()
                        .unwrap()
                        .wire(),
                }))
            }
            Request::ViewStageOpen {
                view: handle,
                expected_revision,
                expected_query_revision,
                kind,
                bytes,
            } => {
                self.check_model_target(owner, &handle, &expected_revision)?;
                self.check_model_query(owner, &handle, expected_query_revision.as_deref())?;
                let state = &self.app.plugins.instances[&owner].application;
                if state.view_stages.len()
                    + state
                        .model_requests
                        .values()
                        .filter(|pending| pending.stage.is_some())
                        .count()
                    >= view::MAX_STAGES
                {
                    return Err(Error::new(Code::LimitExceeded, "View stage limit reached"));
                }
                if bytes == 0 || bytes > view::MAX_MODEL_BYTES {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "View stage byte limit exceeded",
                    ));
                }
                self.reserve_application_payload(owner, bytes + HANDLE_CHARGE)?;
                let stage = view::Stage::new(
                    handle,
                    expected_revision,
                    expected_query_revision,
                    kind,
                    bytes,
                )?;
                let state = &mut self
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application;
                state.next_handle += 1;
                let handle = format!("vs:{}:{}", state.generation, state.next_handle);
                state.view_stages.insert(handle.clone(), stage);
                state.retained_payload += bytes + HANDLE_CHARGE;
                if let Err(error) = self.model_idle(owner, &handle, true) {
                    self.close_model_stage(owner, &handle);
                    self.app
                        .plugins
                        .instances
                        .get_mut(&owner)
                        .unwrap()
                        .application
                        .deadlines
                        .remove(&handle);
                    return Err(error);
                }
                Ok(Some(ResultValue::ViewStage {
                    stage: handle,
                    bytes,
                }))
            }
            Request::ViewStageWrite {
                stage,
                offset,
                text,
            } => {
                let state = &mut self
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application;
                let issued = state
                    .view_stages
                    .get_mut(&stage)
                    .ok_or_else(|| Error::new(Code::NotFound, "Unknown view stage"))?;
                let offset = issued.append(offset, &text)?;
                self.model_idle(owner, &stage, true)?;
                Ok(Some(ResultValue::ViewOffset { offset }))
            }
            Request::ViewStageClose { stage } => {
                if self.close_model_stage(owner, &stage) {
                    self.model_idle(owner, &stage, false)?;
                }
                Ok(Some(ResultValue::Empty(api::Empty {})))
            }
            Request::ViewSnapshotOpen {
                view: handle,
                expected_revision,
            } => {
                self.check_model_target(owner, &handle, &expected_revision)?;
                let state = &self.app.plugins.instances[&owner].application;
                if state.view_snapshots.len() >= view::MAX_SNAPSHOTS {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "View snapshot limit reached",
                    ));
                }
                let encoded = state.views[&handle].encoded.clone();
                let bytes = encoded.len();
                self.reserve_application_payload(owner, bytes + HANDLE_CHARGE)?;
                let state = &mut self
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application;
                state.next_handle += 1;
                let snapshot = format!("vm:{}:{}", state.generation, state.next_handle);
                state.view_snapshots.insert(
                    snapshot.clone(),
                    view::OwnedSnapshot {
                        view: handle,
                        snapshot: view::ReadSnapshot {
                            revision: expected_revision.clone(),
                            encoded,
                        },
                    },
                );
                state.retained_payload += bytes + HANDLE_CHARGE;
                if let Err(error) = self.model_idle(owner, &snapshot, true) {
                    self.close_model_snapshot(owner, &snapshot);
                    self.app
                        .plugins
                        .instances
                        .get_mut(&owner)
                        .unwrap()
                        .application
                        .deadlines
                        .remove(&snapshot);
                    return Err(error);
                }
                let revision = self.app.plugins.instances[&owner]
                    .application
                    .view_snapshots[&snapshot]
                    .snapshot
                    .revision
                    .clone();
                Ok(Some(ResultValue::ViewSnapshot {
                    snapshot,
                    revision,
                    bytes,
                }))
            }
            Request::ViewSnapshotRead {
                snapshot,
                offset,
                limit,
            } => {
                let issued = self.app.plugins.instances[&owner]
                    .application
                    .view_snapshots
                    .get(&snapshot)
                    .ok_or_else(|| Error::new(Code::NotFound, "Unknown view snapshot"))?;
                let (text, next) = issued.snapshot.chunk(offset, limit)?;
                self.model_idle(owner, &snapshot, true)?;
                Ok(Some(ResultValue::ViewChunk {
                    offset,
                    text,
                    eof: next.is_none(),
                }))
            }
            Request::ViewSnapshotClose { snapshot } => {
                if self.close_model_snapshot(owner, &snapshot) {
                    self.model_idle(owner, &snapshot, false)?;
                }
                Ok(Some(ResultValue::Empty(api::Empty {})))
            }
            Request::ViewCreate { model } => self.start_model(
                owner,
                request_id,
                None,
                Input::Model(model),
                None,
                INLINE_PREPARE,
            ),
            Request::ViewPublish {
                view,
                expected_revision,
                expected_query_revision,
                model,
            } => {
                self.check_model_target(owner, &view, &expected_revision)?;
                self.check_model_query(owner, &view, expected_query_revision.as_deref())?;
                self.start_model(
                    owner,
                    request_id,
                    Some(view),
                    Input::Model(model),
                    None,
                    INLINE_PREPARE,
                )
            }
            Request::ViewPatch {
                view,
                expected_revision,
                expected_query_revision,
                header,
                operations,
            } => {
                self.check_model_target(owner, &view, &expected_revision)?;
                self.check_model_query(owner, &view, expected_query_revision.as_deref())?;
                let base = self.app.plugins.instances[&owner].application.views[&view]
                    .model
                    .clone();
                self.start_model(
                    owner,
                    request_id,
                    Some(view),
                    Input::Patch(base, view::Patch { header, operations }),
                    None,
                    LARGE_PREPARE,
                )
            }
            Request::ViewStageCommit { stage } => {
                let issued = self.app.plugins.instances[&owner]
                    .application
                    .view_stages
                    .get(&stage)
                    .ok_or_else(|| Error::new(Code::NotFound, "Unknown view stage"))?;
                self.check_model_target(owner, &issued.view, &issued.expected_revision)?;
                self.check_model_query(
                    owner,
                    &issued.view,
                    issued.expected_query_revision.as_deref(),
                )?;
                let target = issued.view.clone();
                if !issued.is_complete() {
                    return Err(Error::new(
                        Code::InvalidArgument,
                        "View stage is incomplete",
                    ));
                }
                let base = self.app.plugins.instances[&owner].application.views[&target]
                    .model
                    .clone();
                self.start_model(
                    owner,
                    request_id,
                    Some(target),
                    Input::Staged(base, stage.clone()),
                    Some(stage),
                    LARGE_PREPARE,
                )
            }
            _ => unreachable!(),
        }
    }

    fn check_model_target(&self, owner: usize, handle: &str, revision: &str) -> Result<(), Error> {
        let live = self.app.plugins.instances[&owner]
            .application
            .views
            .get(handle)
            .ok_or_else(|| Error::new(Code::NotFound, "Unknown view"))?;
        if self.app.host_buffer_is_closed(live.buffer) {
            return Err(Error::new(Code::Closed, "View is closed"));
        }
        if revision != format!("m:{}", live.revision) {
            return Err(Error::new(Code::Stale, "View model changed"));
        }
        Ok(())
    }

    fn check_model_query(
        &self,
        owner: usize,
        handle: &str,
        expected: Option<&str>,
    ) -> Result<(), Error> {
        view::check_query(
            self.app.plugins.instances[&owner].application.views[handle]
                .query
                .as_ref(),
            expected,
        )
    }

    fn model_admission(
        &self,
        owner: usize,
        target: Option<&str>,
        charge: usize,
    ) -> Result<(), Error> {
        let state = &self.app.plugins.instances[&owner].application;
        if state.model_requests.len() >= MAX_PREPARATIONS
            || target.is_some_and(|target| {
                state
                    .model_requests
                    .values()
                    .any(|pending| pending.view == target)
            })
        {
            return Err(Error::new(
                Code::Busy,
                "View preparation is already pending",
            ));
        }
        if target.is_none()
            && state.views.len()
                + state
                    .model_requests
                    .values()
                    .filter(|pending| pending.creating)
                    .count()
                >= 16
        {
            return Err(Error::new(Code::LimitExceeded, "View limit reached"));
        }
        if let Some(target) = target {
            let live = &state.views[target];
            if live
                .published
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(100))
            {
                return Err(Error::new(
                    Code::Busy,
                    "View publication is paced at ten updates per second",
                ));
            }
        }
        self.reserve_application_payload(owner, charge)
    }

    fn start_model(
        &mut self,
        owner: usize,
        request: &str,
        target: Option<String>,
        input: Input,
        stage: Option<String>,
        charge: usize,
    ) -> Result<Option<ResultValue>, Error> {
        let transferred = stage
            .as_ref()
            .and_then(|stage| {
                self.app.plugins.instances[&owner]
                    .application
                    .view_stages
                    .get(stage)
            })
            .map_or(0, |stage| stage.declared + HANDLE_CHARGE);
        self.model_admission(owner, target.as_deref(), charge.saturating_sub(transferred))?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| Error::new(Code::Unavailable, "Model preparation runtime unavailable"))?;
        let sender = self
            .plugin_events_sender
            .as_ref()
            .ok_or_else(|| Error::new(Code::Unavailable, "Model event channel unavailable"))?
            .clone();
        let slots = self
            .plugin_local_slots
            .get_or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(16)))
            .clone();
        let permit = slots
            .try_acquire_owned()
            .map_err(|_| Error::new(Code::Busy, "Local operation service is busy"))?;
        let input = match input {
            Input::Staged(base, handle) => {
                let issued = self
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application
                    .view_stages
                    .remove(&handle)
                    .expect("admitted stage");
                let kind = issued.kind;
                self.app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application
                    .retained_payload -= transferred;
                self.model_idle(owner, &handle, false)?;
                let text = issued.into_text()?;
                Input::Encoded(base, kind, text)
            }
            input => input,
        };
        let state = &self.app.plugins.instances[&owner].application;
        let old_projection = target
            .as_ref()
            .map(|target| state.views[target].projection.clone());
        let source_charge = target
            .as_ref()
            .map_or(0, |target| state.views[target].charge);
        let expected_query_revision = target.as_ref().and_then(|target| {
            state.views[target]
                .query
                .as_ref()
                .map(|query| format!("qv:{}", query.revision))
        });
        let (buffer, revision, buffer_revision, source) = if let Some(target) = &target {
            let live = &state.views[target];
            (
                Some(live.buffer),
                live.revision,
                self.app.buffers[live.buffer].revision(),
                self.app.buffers[live.buffer].plugin_projection_source(),
            )
        } else {
            (None, 0, 0, PluginProjectionSource::empty())
        };
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        let creating = target.is_none();
        let handle = target.unwrap_or_else(|| {
            state.next_handle += 1;
            format!("v:{}:{}", state.generation, state.next_handle)
        });
        let cancelled = Arc::new(AtomicBool::new(false));
        let generation = state.generation.clone();
        state.retained_payload += charge;
        state.model_requests.insert(
            request.into(),
            view::Pending {
                view: handle,
                creating,
                buffer,
                revision,
                buffer_revision,
                charge,
                source_charge,
                expected_query_revision,
                stage,
                cancelled: cancelled.clone(),
            },
        );
        let request = request.to_owned();
        let work = runtime.spawn_blocking(move || {
            let mut model = input.prepare(&cancelled)?;
            let spans = model.projection.spans.clone();
            let remap = old_projection
                .as_ref()
                .map_or_else(Vec::new, |old| model.projection.remap_from(old));
            if cancelled.load(Ordering::SeqCst) {
                return Err(Error::new(Code::Cancelled, "View preparation cancelled"));
            }
            let text = std::mem::take(
                &mut Arc::get_mut(&mut model.projection)
                    .expect("newly prepared projection")
                    .text,
            );
            let text = source.prepare(text);
            if cancelled.load(Ordering::SeqCst) {
                return Err(Error::new(Code::Cancelled, "View preparation cancelled"));
            }
            Ok(view::Prepared {
                model,
                text,
                spans,
                remap,
            })
        });
        runtime.spawn(async move {
            let result = work.await.unwrap_or_else(|_| {
                Err(Error::new(Code::Internal, "View preparation worker failed"))
            });
            let _ = sender
                .send(plugin::Event {
                    plugin: owner,
                    result: Ok(plugin::ClientMessage::ModelPrepared {
                        generation,
                        request,
                        result,
                        _permit: permit,
                    }),
                })
                .await;
        });
        Ok(None)
    }

    pub(super) fn application_model_result(
        &mut self,
        owner: usize,
        generation: String,
        request: String,
        result: Result<view::Prepared, Error>,
    ) -> anyhow::Result<()> {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        if state.generation != generation {
            return Ok(());
        }
        let Some(pending) = state.model_requests.remove(&request) else {
            return Ok(());
        };
        state.retained_payload -= pending.charge;
        let result = result.and_then(|prepared| self.install_model(owner, pending, prepared));
        self.application_local_reply(owner, request, result)
    }

    fn install_model(
        &mut self,
        owner: usize,
        pending: view::Pending,
        prepared: view::Prepared,
    ) -> Result<ResultValue, Error> {
        if pending.cancelled.load(Ordering::SeqCst) {
            return Err(Error::new(Code::Cancelled, "View preparation cancelled"));
        }
        let model = &prepared.model;
        if model.charge > pending.charge {
            return Err(Error::new(
                Code::LimitExceeded,
                "Prepared view exceeded reserved memory",
            ));
        }
        if let Some(buffer) = pending.buffer {
            self.check_model_target(owner, &pending.view, &format!("m:{}", pending.revision))?;
            self.check_model_query(
                owner,
                &pending.view,
                pending.expected_query_revision.as_deref(),
            )?;
            let live = &self.app.plugins.instances[&owner].application.views[&pending.view];
            if live.buffer != buffer
                || self.app.buffers[buffer].revision() != pending.buffer_revision
            {
                return Err(Error::new(
                    Code::Stale,
                    "View projection changed during preparation",
                ));
            }
            if live
                .published
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(100))
            {
                return Err(Error::new(
                    Code::Busy,
                    "View publication is paced at ten updates per second",
                ));
            }
        }
        if model.model.actions.iter().any(|action| {
            self.app.plugins.instances[&owner]
                .application
                .command_contexts
                .get(action)
                != Some(&api::CommandContext::View)
        }) {
            return Err(Error::new(
                Code::InvalidArgument,
                "View action is not a registered view command",
            ));
        }
        let old_charge = pending.buffer.map_or(0, |_| {
            self.app.plugins.instances[&owner].application.views[&pending.view].charge
        });
        let (mut query, accepted_actions) = pending.buffer.map_or((None, 0), |_| {
            let live = &self.app.plugins.instances[&owner].application.views[&pending.view];
            (live.query.clone(), live.accepted_actions)
        });
        if let Some(query) = &mut query {
            query.pending = false;
        }
        let charge = model.charge + query.as_ref().map_or(0, |_| view::QUERY_CHARGE);
        self.reserve_application_payload(owner, charge.saturating_sub(old_charge))?;
        let buffer = if let Some(buffer) = pending.buffer {
            let old = self.app.plugins.instances[&owner].application.views[&pending.view]
                .projection
                .clone();
            if !self.app.publish_prepared_plugin_view(
                buffer,
                &old,
                model,
                prepared.text,
                prepared.spans,
                &prepared.remap,
            ) {
                return Err(Error::new(
                    Code::Stale,
                    "View projection changed during preparation",
                ));
            }
            buffer
        } else {
            self.app.create_prepared_plugin_view(
                owner,
                &pending.view,
                model,
                prepared.text,
                prepared.spans,
            )
        };
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        state.retained_payload = state.retained_payload - old_charge + charge;
        state.views.insert(
            pending.view.clone(),
            view::View {
                buffer,
                model: model.model.clone(),
                encoded: model.encoded.clone(),
                projection: model.projection.clone(),
                charge,
                query,
                accepted_actions,
                revision: pending.revision + 1,
                published: (!pending.creating).then(std::time::Instant::now),
            },
        );
        self.app.refresh_plugin_view_title(owner, &pending.view);
        Ok(self.model_result(owner, pending.view))
    }

    pub(super) fn model_result(&self, owner: usize, handle: String) -> ResultValue {
        let live = &self.app.plugins.instances[&owner].application.views[&handle];
        let revision = format!("m:{}", live.revision);
        let query = live.query.as_ref().map(view::QueryState::wire);
        if live.encoded.len() < plugin::MAX_BYTES - 4096 {
            ResultValue::View {
                view: handle,
                revision,
                model: live.model.clone(),
                query,
            }
        } else {
            ResultValue::ViewInfo {
                view: handle,
                revision,
                bytes: live.encoded.len(),
                rows: live.model.rows.len(),
                query,
            }
        }
    }

    fn model_idle(&mut self, owner: usize, handle: &str, active: bool) -> Result<(), Error> {
        let previous = self.app.plugins.instances[&owner]
            .application
            .deadlines
            .get(handle)
            .copied();
        if self
            .plugin_send(
                owner,
                plugin::HostMessage::Deadline {
                    token: handle.into(),
                    after_ms: active.then_some(view::IDLE_SECONDS * 1000),
                },
            )
            .is_err()
        {
            let deadlines = &mut self
                .app
                .plugins
                .instances
                .get_mut(&owner)
                .unwrap()
                .application
                .deadlines;
            if let Some(previous) = previous {
                deadlines.insert(handle.into(), previous);
            } else {
                deadlines.remove(handle);
            }
            return Err(Error::new(
                Code::Unavailable,
                "View lifecycle queue unavailable",
            ));
        }
        Ok(())
    }
    fn close_model_stage(&mut self, owner: usize, handle: &str) -> bool {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        let mut removed = false;
        if let Some(stage) = state.view_stages.remove(handle) {
            state.retained_payload -= stage.declared + HANDLE_CHARGE;
            removed = true;
        }
        for pending in state.model_requests.values() {
            if pending.stage.as_deref() == Some(handle) {
                pending.cancelled.store(true, Ordering::SeqCst);
                removed = true;
            }
        }
        removed
    }
    fn close_model_snapshot(&mut self, owner: usize, handle: &str) -> bool {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        if let Some(snapshot) = state.view_snapshots.remove(handle) {
            state.retained_payload -= snapshot.snapshot.encoded.len() + HANDLE_CHARGE;
            true
        } else {
            false
        }
    }
    pub(super) fn model_deadline(&mut self, owner: usize, handle: &str) -> bool {
        let state = &self.app.plugins.instances[&owner].application;
        if state.view_stages.contains_key(handle) {
            self.close_model_stage(owner, handle);
            true
        } else if state.view_snapshots.contains_key(handle) {
            self.close_model_snapshot(owner, handle);
            true
        } else {
            false
        }
    }
    pub(super) fn retire_view_models(&mut self, owner: usize, view: &str) -> anyhow::Result<()> {
        let state = &self.app.plugins.instances[&owner].application;
        let stages = state
            .view_stages
            .iter()
            .filter(|(_, stage)| stage.view == view)
            .map(|(handle, _)| handle.clone())
            .collect::<Vec<_>>();
        let snapshots = state
            .view_snapshots
            .iter()
            .filter(|(_, snapshot)| snapshot.view == view)
            .map(|(handle, _)| handle.clone())
            .collect::<Vec<_>>();
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        for pending in state.model_requests.values_mut() {
            if pending.view == view {
                pending.cancelled.store(true, Ordering::SeqCst);
                // The worker still owns its captured source after the live view
                // retires. Transfer that reservation before removing view.charge.
                state.retained_payload += pending.source_charge;
                pending.charge += pending.source_charge;
                pending.source_charge = 0;
            }
        }
        for handle in stages {
            self.close_model_stage(owner, &handle);
            self.model_idle(owner, &handle, false)
                .map_err(|error| anyhow::anyhow!(error.message))?;
        }
        for handle in snapshots {
            self.close_model_snapshot(owner, &handle);
            self.model_idle(owner, &handle, false)
                .map_err(|error| anyhow::anyhow!(error.message))?;
        }
        Ok(())
    }
}
