// SPDX-License-Identifier: MPL-2.0

//! Lease ownership, monotonic expiry and explicit cleanup acknowledgement.
use super::WorkspaceHost;
use crate::plugin::{self, activity, application as api};
use api::{Error, ErrorCode as Code, Request, ResultValue};
use std::time::{Duration, Instant};

pub(super) fn is_activity_request(request: &Request) -> bool {
    matches!(
        request,
        Request::ActivityAcquire { .. }
            | Request::ActivityRenew { .. }
            | Request::ActivityGet { .. }
            | Request::ActivityRelease { .. }
            | Request::ActivityCancel { .. }
    )
}
struct Outcome {
    result: Result<ResultValue, Error>,
    timer: Option<(String, Option<u64>)>,
    event: Option<activity::Cancelled>,
}
impl Outcome {
    fn reply(result: Result<ResultValue, Error>) -> Self {
        Self {
            result,
            timer: None,
            event: None,
        }
    }
}

impl WorkspaceHost {
    pub(super) fn application_activity_request(
        &mut self,
        owner: usize,
        request_id: String,
        request: Request,
    ) -> anyhow::Result<()> {
        if !self.app.plugins.instances[&owner]
            .application
            .capabilities
            .contains("activity")
        {
            return self.application_local_reply(
                owner,
                request_id,
                Err(Error::new(
                    Code::CapabilityDenied,
                    "Activity capability was not granted",
                )),
            );
        }
        // A late release/renew cannot overtake an already missed cleanup deadline.
        if let Request::ActivityGet { lease }
        | Request::ActivityRenew { lease, .. }
        | Request::ActivityRelease { lease }
        | Request::ActivityCancel { lease } = &request
            && let Some(issued) = self.app.plugins.instances[&owner]
                .application
                .activities
                .get(lease)
            && issued.info.state == activity::State::Cancelling
            && issued.deadline <= Instant::now()
        {
            anyhow::bail!("application did not acknowledge activity cancellation");
        }
        let outcome = self.activity_operation(owner, request);
        if let Some((lease, milliseconds)) = outcome.timer {
            self.activity_timer(owner, &lease, milliseconds)?;
        }
        self.application_local_reply(owner, request_id, outcome.result)?;
        if let Some(event) = outcome.event {
            self.activity_event(owner, event)?;
        }
        Ok(())
    }

    fn activity_operation(&mut self, owner: usize, request: Request) -> Outcome {
        if let Request::ActivityAcquire {
            title,
            duration_seconds,
        } = request
        {
            if let Err(error) = activity::validate_title(&title)
                .and_then(|()| activity::validate_duration(duration_seconds))
            {
                return Outcome::reply(Err(error));
            }
            let state = &self.app.plugins.instances[&owner].application;
            if state.activities.len() >= activity::MAX_LEASES {
                return Outcome::reply(Err(Error::new(
                    Code::LimitExceeded,
                    "Activity lease limit reached; release an existing lease",
                )));
            }
            if let Err(error) = self.reserve_application_payload(owner, activity::LEASE_CHARGE) {
                return Outcome::reply(Err(error));
            }
            let state = &mut self
                .app
                .plugins
                .instances
                .get_mut(&owner)
                .unwrap()
                .application;
            let Some(next) = state.next_handle.checked_add(1) else {
                return Outcome::reply(Err(Error::new(
                    Code::LimitExceeded,
                    "Activity handle identities exhausted",
                )));
            };
            state.next_handle = next;
            let handle = format!("a:{}:{next}", state.generation);
            let info = activity::Info {
                lease: handle.clone(),
                title,
                state: activity::State::Active,
                duration_seconds,
            };
            state.activities.insert(
                handle.clone(),
                activity::Lease {
                    info: info.clone(),
                    deadline: Instant::now() + Duration::from_secs(duration_seconds),
                },
            );
            state.retained_payload += activity::LEASE_CHARGE;
            self.app.plugins.presentation_dirty = true;
            return Outcome {
                result: Ok(ResultValue::Activity(info)),
                timer: Some((handle, Some(duration_seconds * 1000))),
                event: None,
            };
        }
        let (handle, duration) = match &request {
            Request::ActivityRenew {
                lease,
                duration_seconds,
            } => (lease, Some(*duration_seconds)),
            Request::ActivityGet { lease }
            | Request::ActivityRelease { lease }
            | Request::ActivityCancel { lease } => (lease, None),
            _ => {
                return Outcome::reply(Err(Error::new(
                    Code::Unsupported,
                    "Unknown activity request",
                )));
            }
        };
        if let Some(duration) = duration
            && let Err(error) = activity::validate_duration(duration)
        {
            return Outcome::reply(Err(error));
        }
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        if matches!(request, Request::ActivityRelease { .. }) {
            let prefix = format!("a:{}:", state.generation);
            let owned = handle
                .strip_prefix(&prefix)
                .and_then(|index| {
                    let value = index.parse::<u64>().ok()?;
                    (value > 0 && value <= state.next_handle && value.to_string() == index)
                        .then_some(())
                })
                .is_some();
            if !owned {
                return Outcome::reply(Err(Error::new(Code::NotFound, "Unknown activity lease")));
            }
            if state.activities.remove(handle).is_some() {
                state.retained_payload -= activity::LEASE_CHARGE;
                self.app.plugins.presentation_dirty = true;
                return Outcome {
                    result: Ok(ResultValue::Empty(api::Empty {})),
                    timer: Some((handle.clone(), None)),
                    event: None,
                };
            }
            return Outcome::reply(Ok(ResultValue::Empty(api::Empty {})));
        }
        let Some(issued) = state.activities.get_mut(handle) else {
            return Outcome::reply(Err(Error::new(Code::NotFound, "Unknown activity lease")));
        };
        let now = Instant::now();
        let event = if issued.info.state == activity::State::Active && issued.deadline <= now {
            issued.cancel(activity::Reason::Expired, now)
        } else if matches!(request, Request::ActivityCancel { .. }) {
            issued.cancel(activity::Reason::Cancelled, now)
        } else {
            None
        };
        let mut timer = event
            .as_ref()
            .map(|_| (handle.clone(), Some(activity::CLEANUP_SECONDS * 1000)));
        if event.is_some() {
            self.app.plugins.presentation_dirty = true;
        }
        let result = if let Some(duration_seconds) = duration {
            if issued.info.state == activity::State::Cancelling {
                Err(Error::new(
                    Code::Cancelled,
                    "Activity lease is cancelling; acknowledge cleanup with activity.release",
                ))
            } else {
                issued.info.duration_seconds = duration_seconds;
                issued.deadline = now + Duration::from_secs(duration_seconds);
                timer = Some((handle.clone(), Some(duration_seconds * 1000)));
                self.app.plugins.presentation_dirty = true;
                Ok(ResultValue::Activity(issued.info.clone()))
            }
        } else {
            Ok(ResultValue::Activity(issued.info.clone()))
        };
        Outcome {
            result,
            timer,
            event,
        }
    }

    fn activity_timer(
        &mut self,
        owner: usize,
        lease: &str,
        after_ms: Option<u64>,
    ) -> anyhow::Result<()> {
        self.plugin_send(
            owner,
            plugin::HostMessage::Deadline {
                token: lease.into(),
                after_ms,
            },
        )?;
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        if let Some(issued) = state.activities.get_mut(lease)
            && let Some(deadline) = state.deadlines.get(lease)
        {
            issued.deadline = *deadline;
        }
        Ok(())
    }
    fn activity_event(&mut self, owner: usize, event: activity::Cancelled) -> anyhow::Result<()> {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        state.sequence += 1;
        let sequence = format!("e:{}", state.sequence);
        self.application_send(
            owner,
            api::HostMessage::Event {
                sequence,
                event: "activity.cancel_requested",
                data: api::EventData::ActivityCancelled(event),
            },
        )
    }
    pub(super) fn activity_deadline(&mut self, owner: usize, token: &str) -> anyhow::Result<bool> {
        if !token.starts_with("a:") {
            return Ok(false);
        }
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        let Some(issued) = state.activities.get_mut(token) else {
            return Ok(true);
        };
        let now = Instant::now();
        if issued.deadline > now {
            return Ok(true);
        }
        anyhow::ensure!(
            issued.info.state != activity::State::Cancelling,
            "application did not acknowledge activity cancellation"
        );
        let event = issued.cancel(activity::Reason::Expired, now).unwrap();
        self.app.plugins.presentation_dirty = true;
        self.activity_timer(owner, token, Some(activity::CLEANUP_SECONDS * 1000))?;
        self.activity_event(owner, event)?;
        Ok(true)
    }
}
