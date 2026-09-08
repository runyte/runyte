// SPDX-License-Identifier: MPL-2.0
use super::filesystem::{complete, local_service, response};
use super::*;

#[tokio::test]
async fn stat_is_shallow_revision_checked_and_does_not_retain_a_directory_handle() {
    let (root, mut host) = host();
    let path = root.path().join("é猫.txt");
    std::fs::write(&path, "é猫").unwrap();
    let mut output = setup(&mut host, 0, &["filesystem"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    request(
        &mut host,
        0,
        1,
        api::Request::FilesystemStat {
            path: "é猫.txt".into(),
            expected_revision: None,
        },
    );
    let api::ResultValue::Stat(stat) = complete(&mut host, &mut events, &mut output).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(
        (stat.path.as_str(), stat.kind, stat.bytes),
        ("é猫.txt", "file", 5)
    );
    request(
        &mut host,
        0,
        2,
        api::Request::FilesystemStat {
            path: "é猫.txt".into(),
            expected_revision: Some(stat.revision.clone()),
        },
    );
    assert!(matches!(
        complete(&mut host, &mut events, &mut output).await.unwrap(),
        api::ResultValue::Stat(_)
    ));
    std::fs::write(&path, "changed").unwrap();
    request(
        &mut host,
        0,
        3,
        api::Request::FilesystemStat {
            path: "é猫.txt".into(),
            expected_revision: Some(stat.revision),
        },
    );
    assert_eq!(
        complete(&mut host, &mut events, &mut output)
            .await
            .unwrap_err()
            .code,
        api::ErrorCode::Stale
    );
    let directory = root.path().join("many");
    std::fs::create_dir(&directory).unwrap();
    for n in 0..1025 {
        std::fs::write(directory.join(n.to_string()), "").unwrap();
    }
    request(
        &mut host,
        0,
        4,
        api::Request::FilesystemStat {
            path: "many".into(),
            expected_revision: None,
        },
    );
    assert!(
        matches!(complete(&mut host, &mut events, &mut output).await.unwrap(), api::ResultValue::Stat(stat) if stat.kind == "directory")
    );
    let state = &host.app.plugins.instances[&0].application;
    assert!(state.directories.is_empty());
    assert_eq!(state.retained_payload, 0);
}
#[tokio::test]
async fn stat_requires_capability_and_rejects_escaped_or_absent_paths() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["filesystem"]);
    next(&mut output);
    let mut other = setup(&mut host, 1, &[]);
    next(&mut other);
    let mut events = local_service(&mut host);
    request(
        &mut host,
        1,
        1,
        api::Request::FilesystemStat {
            path: ".".into(),
            expected_revision: None,
        },
    );
    assert_eq!(
        response(&mut other).unwrap_err().code,
        api::ErrorCode::CapabilityDenied
    );
    for (n, path, code) in [
        (1, "../outside", api::ErrorCode::InvalidArgument),
        (2, "missing", api::ErrorCode::NotFound),
    ] {
        request(
            &mut host,
            0,
            n,
            api::Request::FilesystemStat {
                path: path.into(),
                expected_revision: None,
            },
        );
        assert_eq!(
            complete(&mut host, &mut events, &mut output)
                .await
                .unwrap_err()
                .code,
            code
        );
    }
}
#[cfg(unix)]
#[tokio::test]
async fn stat_resolves_only_symlink_targets_inside_the_workspace() {
    let (root, mut host) = host();
    let outside = crate::test_support::TestRuntimeRoot::new("plugin-stat-outside").unwrap();
    std::fs::write(root.path().join("file"), "text").unwrap();
    std::os::unix::fs::symlink(root.path().join("file"), root.path().join("alias")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("outside")).unwrap();
    let mut output = setup(&mut host, 0, &["filesystem"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    request(
        &mut host,
        0,
        1,
        api::Request::FilesystemStat {
            path: "alias".into(),
            expected_revision: None,
        },
    );
    assert!(
        matches!(complete(&mut host, &mut events, &mut output).await.unwrap(), api::ResultValue::Stat(stat) if stat.kind == "file" && stat.bytes == 4)
    );
    request(
        &mut host,
        0,
        2,
        api::Request::FilesystemStat {
            path: "outside".into(),
            expected_revision: None,
        },
    );
    assert_eq!(
        complete(&mut host, &mut events, &mut output)
            .await
            .unwrap_err()
            .code,
        api::ErrorCode::InvalidArgument
    );
}
