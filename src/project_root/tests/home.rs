// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::{test_support::TestRuntimeRoot, workspace_state::StartupSafety};

#[test]
fn exact_homes_share_initialization_without_becoming_discovery_markers() {
    let fixture = TestRuntimeRoot::new("home-policy").unwrap();
    let environment = fixture.create_private_dir("environment").unwrap();
    let account = fixture.create_private_dir("account").unwrap();
    let state = fixture.join("state/runyte");
    let safety = StartupSafety::capture_with_homes(
        &[],
        || Some(state.clone()),
        &[environment.clone(), account.clone()],
    );
    for home in [&environment, &account] {
        let found = discover_with_home_safety(home, Path::new(".runyte"), &safety)
            .unwrap()
            .unwrap();
        assert_eq!(found.origin, WorkspaceOrigin::Home);
        assert_eq!(&found.root, home);
        let expected = StateLocation::out_of_tree(home, &state);
        for configured in [Path::new(".runyte"), fixture.path()] {
            assert_eq!(
                resolve_state_location_with_safety(home, configured, &safety).unwrap(),
                expected
            );
            assert_eq!(
                initialize_with_safety(home, configured, &safety).unwrap(),
                *home
            );
            assert!(!home.join(".runyte").exists());
        }
        assert!(expected.as_path().is_dir());
        assert!(discover(home, Path::new(".runyte")).unwrap().is_none());
        let nested = home.join("scratch");
        fs::create_dir(&nested).unwrap();
        assert!(
            discover_with_home_safety(&nested, Path::new(".runyte"), &safety)
                .unwrap()
                .is_none()
        );
        adopt_with_safety(
            &nested,
            Path::new(".runyte"),
            &safety,
            &[],
            &[],
            WorkspaceRequirement::new(WorkspaceMode::Ide, false),
        )
        .unwrap();
        assert!(nested.join(".runyte").is_dir());
    }
}

#[test]
fn home_markers_and_checkouts_preserve_origin_without_creating_home_state() {
    let fixture = TestRuntimeRoot::new("home-precedence").unwrap();
    let home = fixture.create_private_dir("home").unwrap();
    let safety = StartupSafety::capture_with_homes(
        &[],
        || Some(fixture.join("state/runyte")),
        std::slice::from_ref(&home),
    );
    for (vcs, marker) in [(".git", "HEAD"), (".jj", "repo"), (".svn", "wc.db")] {
        fs::create_dir(home.join(vcs)).unwrap();
        fs::write(home.join(vcs).join(marker), "fixture").unwrap();
        let found = discover_with_home_safety(&home, Path::new(".runyte"), &safety)
            .unwrap()
            .unwrap();
        assert!(matches!(found.origin, WorkspaceOrigin::VersionControl(_)));
        assert!(matches!(
            resolve_state_location_with_safety(&home, Path::new(".runyte"), &safety).unwrap(),
            StateLocation::OutOfTree(_)
        ));
        initialize_with_safety(&home, Path::new(".runyte"), &safety).unwrap();
        assert!(!home.join(".runyte").exists());
        fs::remove_dir_all(home.join(vcs)).unwrap();
    }
    fs::create_dir(home.join(".runyte")).unwrap();
    assert_eq!(
        discover_with_home_safety(&home, Path::new(".runyte"), &safety)
            .unwrap()
            .unwrap()
            .origin,
        WorkspaceOrigin::StateDirectory
    );
    assert_eq!(
        resolve_state_location_with_safety(&home, Path::new(".runyte"), &safety).unwrap(),
        StateLocation::in_tree(&home, Path::new(".runyte"))
    );
    fs::create_dir(home.join(".git")).unwrap();
    fs::write(home.join(".git/HEAD"), "fixture").unwrap();
    assert!(matches!(
        exact_origin_with_safety(&home, Path::new(".runyte"), &safety).unwrap(),
        WorkspaceOrigin::VersionControl(_)
    ));
    assert!(matches!(
        resolve_state_location_with_safety(&home, Path::new(".runyte"), &safety).unwrap(),
        StateLocation::InTree(_)
    ));
}

#[test]
fn home_without_a_state_root_refuses_without_a_marker() {
    let fixture = TestRuntimeRoot::new("home-no-state").unwrap();
    let home = fixture.create_private_dir("home").unwrap();
    let safety = StartupSafety::capture_with_homes(&[], || None, std::slice::from_ref(&home));
    assert!(initialize_with_safety(&home, Path::new(".runyte"), &safety).is_err());
    assert!(!home.join(".runyte").exists());
}

#[cfg(unix)]
#[test]
fn canonical_home_aliases_refresh_without_account_fallback_and_shared_homes_refuse() {
    use std::{
        os::unix::fs::{PermissionsExt, symlink},
        sync::Arc,
    };
    let fixture = TestRuntimeRoot::new("home-alias").unwrap();
    let home = fixture.create_private_dir("home").unwrap();
    let next = fixture.create_private_dir("next").unwrap();
    let alias = fixture.join("alias");
    symlink(&home, &alias).unwrap();
    let safety = Arc::new(StartupSafety::capture_with_homes(
        &[],
        || Some(fixture.join("state/runyte")),
        std::slice::from_ref(&alias),
    ));
    assert_eq!(
        discover_with_home_safety(&alias, Path::new(".runyte"), &safety)
            .unwrap()
            .unwrap()
            .root,
        home
    );
    safety.close();
    fs::remove_file(&alias).unwrap();
    symlink(&next, &alias).unwrap();
    let fresh = StartupSafety::for_startup(Some(safety), || panic!("must retain injected roots"));
    assert!(
        discover_with_home_safety(&home, Path::new(".runyte"), &fresh)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        discover_with_home_safety(&alias, Path::new(".runyte"), &fresh)
            .unwrap()
            .unwrap()
            .root,
        next
    );
    fs::set_permissions(&next, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(discover_with_home_safety(&next, Path::new(".runyte"), &fresh).is_err());
    assert!(!next.join(".runyte").exists());
}
