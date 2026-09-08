// SPDX-License-Identifier: MPL-2.0

use super::filesystem::{applied, complete, foreground, local_service, response, started};
use super::*;
use crate::input::KeyStroke;
use crate::plugin::staging::{MAX_BYTES, PREPARE_CHARGE, STAGING_CHARGE};

struct DownloadCase {
    root: TestRuntimeRoot,
    host: WorkspaceHost,
    events: mpsc::Receiver<Event>,
    output: mpsc::Receiver<HostMessage>,
    serial: u64,
    job: String,
}

impl DownloadCase {
    fn new() -> Self {
        let (root, mut host) = host();
        let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
        next(&mut output);
        let events = local_service(&mut host);
        let (api::ResultValue::Job(job), _) = host
            .application_request(
                0,
                api::Request::JobCreate {
                    title: "Download fixture".into(),
                    deadline_seconds: 60,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        Self {
            root,
            host,
            events,
            output,
            serial: 0,
            job: job.job,
        }
    }
    fn send(&mut self, operation: api::Request) {
        self.serial += 1;
        request(&mut self.host, 0, self.serial, operation);
    }
    async fn result(&mut self) -> Result<api::ResultValue, api::Error> {
        complete(&mut self.host, &mut self.events, &mut self.output).await
    }
    async fn create(&mut self, bytes: &[u8]) -> (String, String) {
        self.send(api::Request::StagingCreate {
            job: self.job.clone(),
            bytes: bytes.len(),
        });
        let api::ResultValue::Staging { staging, path } = self.result().await.unwrap() else {
            panic!()
        };
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        std::fs::write(&path, bytes).unwrap();
        (staging, path)
    }
    async fn prepare(&mut self, staging: &str, bytes: &[u8]) {
        self.send(api::Request::FilesystemList {
            path: ".".into(),
            offset: 0,
            limit: 128,
            expected_revision: None,
        });
        let api::ResultValue::Directory {
            directory,
            revision,
            ..
        } = self.result().await.unwrap()
        else {
            panic!()
        };
        self.send(api::Request::StagingPrepare {
            staging: staging.into(),
            directory,
            expected_revision: revision,
            destination: "猫.bin".into(),
            sha256: crate::hash::sha256_hex(bytes),
        });
    }
    fn finish(&mut self, state: api::TerminalState) {
        self.host
            .application_request(
                0,
                api::Request::JobFinish {
                    job: self.job.clone(),
                    state,
                },
            )
            .unwrap();
    }
    fn state(&self) -> &api::Instance {
        &self.host.app.plugins.instances[&0].application
    }
}

#[tokio::test]
async fn binary_download_uses_sealed_bytes_and_native_confirmation() {
    use std::io::{Seek, Write};
    let mut case = DownloadCase::new();
    let bytes = b"\0\xff\xfe\r\nUTF-8 is not required\0";
    let (stage, path) = case.create(bytes).await;
    let mut retained = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    case.prepare(&stage, bytes).await;
    let api::ResultValue::FilesystemPlan { plan, operations } = case.result().await.unwrap() else {
        panic!()
    };
    assert!(!operations.join("\n").contains(".runyte"));
    assert!(case.state().staging.is_empty());
    assert!(case.state().staging_plans.contains_key(&plan));
    let invocation = foreground(&mut case.host, &mut case.output);
    case.send(api::Request::FilesystemApply {
        plan: plan.clone(),
        invocation,
    });
    response(&mut case.output).unwrap();
    case.finish(api::TerminalState::Succeeded);
    assert!(case.host.app.fs_confirmation.is_some());
    assert!(!case.root.path().join("猫.bin").exists());
    retained.rewind().unwrap();
    retained.write_all(b"MUTATED original inode").unwrap();
    case.host
        .app
        .handle_key(KeyStroke::parse("Enter").unwrap())
        .unwrap();
    case.host.sync_plugin_observers();
    started(&mut case.output);
    assert_eq!(
        applied(&mut case.host, &mut case.events, &mut case.output)
            .await
            .state,
        "succeeded"
    );
    assert_eq!(
        std::fs::read(case.root.path().join("猫.bin")).unwrap(),
        bytes
    );
    retained.rewind().unwrap();
    retained.write_all(b"AFTER publication").unwrap();
    assert_eq!(
        std::fs::read(case.root.path().join("猫.bin")).unwrap(),
        bytes
    );
}

#[tokio::test]
async fn download_confirmation_cancel_and_collision_preserve_destination() {
    for collision in [false, true] {
        let mut case = DownloadCase::new();
        let (stage, _) = case.create(b"").await;
        case.prepare(&stage, b"").await;
        let api::ResultValue::FilesystemPlan { plan, .. } = case.result().await.unwrap() else {
            panic!()
        };
        let invocation = foreground(&mut case.host, &mut case.output);
        case.send(api::Request::FilesystemApply { plan, invocation });
        response(&mut case.output).unwrap();
        case.finish(api::TerminalState::Succeeded);
        if collision {
            std::fs::write(case.root.path().join("猫.bin"), b"keep existing").unwrap();
        }
        case.host
            .app
            .handle_key(KeyStroke::parse(if collision { "Enter" } else { "Esc" }).unwrap())
            .unwrap();
        case.host.sync_plugin_observers();
        if collision {
            started(&mut case.output);
            assert_eq!(
                applied(&mut case.host, &mut case.events, &mut case.output)
                    .await
                    .state,
                "failed"
            );
            assert_eq!(
                std::fs::read(case.root.path().join("猫.bin")).unwrap(),
                b"keep existing"
            );
        } else {
            assert!(matches!(
                next(&mut case.output),
                api::HostMessage::Event {
                    event: "filesystem.finished",
                    ..
                }
            ));
            assert!(!case.root.path().join("猫.bin").exists());
        }
    }
}

#[tokio::test]
async fn download_finish_and_close_overtake_queued_sealing_without_publishing_plan() {
    for close in [false, true] {
        let mut case = DownloadCase::new();
        let (stage, _) = case.create(b"abc").await;
        case.prepare(&stage, b"abc").await;
        let event = case.events.recv().await.unwrap();
        assert_eq!(
            case.state().retained_payload,
            STAGING_CHARGE + PREPARE_CHARGE + crate::plugin::filesystem::DIRECTORY_CHARGE
        );
        if close {
            case.send(api::Request::StagingClose { staging: stage });
            response(&mut case.output).unwrap();
        } else {
            case.finish(api::TerminalState::Failed);
        }
        case.host.handle_plugin_event(event);
        assert_eq!(
            response(&mut case.output).unwrap_err().code,
            api::ErrorCode::Cancelled
        );
        assert!(case.state().staging.is_empty());
        assert!(case.state().plans.is_empty());
        assert_eq!(
            case.state().retained_payload,
            crate::plugin::filesystem::DIRECTORY_CHARGE
        );
        assert!(!case.root.path().join("猫.bin").exists());
    }
}

#[tokio::test]
async fn download_job_end_retires_unpresented_plan_and_pending_creation() {
    let mut case = DownloadCase::new();
    let (stage, _) = case.create(b"abc").await;
    case.prepare(&stage, b"abc").await;
    case.result().await.unwrap();
    assert_eq!(case.state().plans.len(), 1);
    case.finish(api::TerminalState::Succeeded);
    assert!(case.state().plans.is_empty());
    assert!(case.state().staging_plans.is_empty());

    let mut case = DownloadCase::new();
    case.send(api::Request::StagingCreate {
        job: case.job.clone(),
        bytes: 0,
    });
    let event = case.events.recv().await.unwrap();
    case.finish(api::TerminalState::Failed);
    case.host.handle_plugin_event(event);
    assert_eq!(
        response(&mut case.output).unwrap_err().code,
        api::ErrorCode::Cancelled
    );
    assert!(case.state().staging.is_empty());
    assert_eq!(case.state().retained_payload, 0);
}

#[tokio::test]
async fn stopped_download_owner_keeps_worker_charge_until_result_is_discarded() {
    let mut case = DownloadCase::new();
    let (stage, _) = case.create(b"abc").await;
    case.prepare(&stage, b"abc").await;
    let event = case.events.recv().await.unwrap();
    case.host.stop_plugin(0, "stopped by user");
    assert_eq!(case.host.app.plugins.orphaned_payload, PREPARE_CHARGE);
    assert_eq!(case.host.plugin_local_orphans.len(), 1);
    case.host.handle_plugin_event(event);
    assert_eq!(case.host.app.plugins.orphaned_payload, 0);
    assert!(case.host.plugin_local_orphans.is_empty());
    assert!(!case.root.path().join("猫.bin").exists());
}

#[tokio::test]
async fn download_stage_limits_ownership_and_digest_failure_are_recoverable() {
    let mut case = DownloadCase::new();
    case.send(api::Request::StagingCreate {
        job: case.job.clone(),
        bytes: MAX_BYTES + 1,
    });
    assert_eq!(
        response(&mut case.output).unwrap_err().code,
        api::ErrorCode::LimitExceeded
    );
    let (stage, _) = case.create(b"abc").await;
    let (_second, _) = case.create(b"").await;
    case.send(api::Request::StagingCreate {
        job: case.job.clone(),
        bytes: 0,
    });
    assert_eq!(
        response(&mut case.output).unwrap_err().code,
        api::ErrorCode::LimitExceeded
    );
    let mut other = setup(&mut case.host, 1, &["filesystem", "jobs"]);
    next(&mut other);
    request(
        &mut case.host,
        1,
        1,
        api::Request::StagingPrepare {
            staging: stage.clone(),
            directory: "foreign".into(),
            expected_revision: "d:1".into(),
            destination: "bad.bin".into(),
            sha256: crate::hash::sha256_hex(b"abc"),
        },
    );
    assert_eq!(
        response(&mut other).unwrap_err().code,
        api::ErrorCode::NotFound
    );
    case.prepare(&stage, b"different").await;
    assert_eq!(
        case.result().await.unwrap_err().code,
        api::ErrorCode::Conflict
    );
    assert!(!case.state().staging[&stage].busy);
    case.prepare(&stage, b"abc").await;
    assert!(matches!(
        case.result().await.unwrap(),
        api::ResultValue::FilesystemPlan { .. }
    ));
    case.finish(api::TerminalState::Cancelled);
    assert!(case.state().staging.is_empty());
    assert!(case.state().plans.is_empty());
}
