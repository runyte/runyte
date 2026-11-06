// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn origin_handoff_is_bounded_and_does_not_rediscover_a_resolved_host() {
    let root = crate::test_support::TestRuntimeRoot::new("workspace-origin").unwrap();
    let state = Path::new(".runyte");
    for origin in [
        WorkspaceOrigin::VersionControl(VersionControl::Git),
        WorkspaceOrigin::VersionControl(VersionControl::Jujutsu),
        WorkspaceOrigin::VersionControl(VersionControl::Subversion),
        WorkspaceOrigin::StateDirectory,
        WorkspaceOrigin::Home,
        WorkspaceOrigin::Created(CreatedBy::Adoption),
        WorkspaceOrigin::Created(CreatedBy::Init),
        WorkspaceOrigin::Created(CreatedBy::AttachDirectory),
    ] {
        assert_eq!(
            WorkspaceOrigin::from_startup_value(origin.startup_value()),
            Some(origin)
        );
        let missing = root.join("missing");
        assert_eq!(
            startup_origin(&missing, state, Some(&origin.startup_handoff(&missing))).unwrap(),
            origin
        );
    }
    assert_eq!(WorkspaceOrigin::from_startup_value("unknown"), None);
    assert_eq!(
        exact_origin(root.path(), state).unwrap(),
        WorkspaceOrigin::Created(CreatedBy::AttachDirectory)
    );
    fs::create_dir(root.join(".runyte")).unwrap();
    assert_eq!(
        exact_origin(root.path(), state).unwrap(),
        WorkspaceOrigin::StateDirectory
    );
    let captured = WorkspaceOrigin::Created(CreatedBy::Adoption)
        .startup_handoff(&root.join("other-workspace"));
    assert_eq!(
        startup_origin(root.path(), state, Some(&captured)).unwrap(),
        WorkspaceOrigin::StateDirectory
    );
    assert_eq!(
        startup_origin(root.path(), state, Some(&"a".repeat(128))).unwrap(),
        WorkspaceOrigin::StateDirectory
    );
    assert_eq!(
        startup_origin(root.path(), state, Some("adoption")).unwrap(),
        WorkspaceOrigin::StateDirectory
    );
    fs::create_dir_all(root.join(".jj/repo")).unwrap();
    assert_eq!(
        exact_origin(root.path(), state).unwrap(),
        WorkspaceOrigin::VersionControl(VersionControl::Jujutsu)
    );
    assert_eq!(
        startup_origin(root.path(), state, None).unwrap(),
        WorkspaceOrigin::VersionControl(VersionControl::Jujutsu)
    );
}
