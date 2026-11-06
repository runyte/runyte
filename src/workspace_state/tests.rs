// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;

#[test]
fn platform_roots_use_effective_account_inputs_and_only_unix_honors_xdg() {
    let fixture = TestRuntimeRoot::new("state-map").unwrap();
    let home = fixture.join("account");
    let local = fixture.join("local");
    let explicit = fixture.join("explicit");
    assert_eq!(
        root_for(StatePlatform::Unix, Some(&explicit), None, None),
        Some(explicit.join("runyte"))
    );
    assert_eq!(
        root_for(
            StatePlatform::Unix,
            Some(Path::new("relative")),
            Some(&home),
            None
        ),
        Some(home.join(".local/state/runyte"))
    );
    assert_eq!(root_for(StatePlatform::Unix, None, None, None), None);
    assert_eq!(
        root_for(StatePlatform::Unix, Some(Path::new("")), Some(&home), None),
        Some(home.join(".local/state/runyte"))
    );
    assert_eq!(
        root_for(
            StatePlatform::MacOs,
            None,
            Some(Path::new("relative")),
            None
        ),
        None
    );
    assert_eq!(
        root_for(
            StatePlatform::Windows,
            None,
            None,
            Some(Path::new("relative"))
        ),
        None
    );
    assert_eq!(
        root_for(StatePlatform::MacOs, Some(&explicit), Some(&home), None),
        Some(home.join("Library/Application Support/runyte"))
    );
    assert_eq!(
        root_for(StatePlatform::MacOs, Some(&explicit), None, None),
        None
    );
    assert_eq!(
        root_for(
            StatePlatform::Windows,
            Some(&explicit),
            Some(&home),
            Some(&local)
        ),
        Some(local.join("runyte"))
    );
    assert_eq!(
        root_for(StatePlatform::Windows, None, Some(&home), None),
        None
    );
}

#[test]
fn in_tree_selection_does_not_invoke_account_or_state_root_resolution() {
    let project = Path::new("/project");
    let resolve =
        || -> Result<PathBuf> { panic!("an ordinary workspace must not resolve per-user state") };
    let location =
        StateLocation::resolve_with(project, Path::new(".runyte"), false, resolve).unwrap();
    assert_eq!(location, StateLocation::InTree(project.join(".runyte")));
    let absolute = std::env::temp_dir().join("explicit-state");
    assert_eq!(
        StateLocation::in_tree(project, &absolute).as_path(),
        absolute
    );
    assert!(
        StateLocation::resolve_with(project, Path::new(".runyte"), true, || anyhow::bail!(
            "no account root"
        ))
        .is_err()
    );
}

#[test]
fn private_workspace_storage_contains_real_state_consumers_without_project_writes() {
    let fixture = TestRuntimeRoot::new("state-use").unwrap();
    let project = fixture.create_private_dir("project").unwrap();
    let root = fixture.join("state/runyte");
    let location =
        StateLocation::resolve_with(&project, Path::new(".runyte"), true, || Ok(root.clone()))
            .unwrap();
    assert_eq!(
        location.as_path(),
        root.join("workspaces")
            .join(crate::workspace::workspace_id(&project))
    );
    location
        .prepare(&[fixture.join("config"), fixture.join("cache")])
        .unwrap();
    let mut config = crate::config::Config::default();
    config.workspace.state = location.as_path().to_owned();
    config.lsp.enable = false;
    let app = crate::app::App::new_in_project(config, None, &project).unwrap();
    assert_eq!(app.state_root, location.as_path());
    let image = crate::pasted_image::store(
        &app.state_root,
        b"fixture-image",
        crate::pasted_image::ImageFormat::Png,
    )
    .unwrap();
    assert!(image.starts_with(location.as_path().join("cache/images")));
    let download = crate::plugin::staging::Download::create(
        &app.state_root,
        0,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        download
            .path()
            .starts_with(location.as_path().join("cache/plugin-downloads"))
    );
    let log = crate::log::default_path(&app.state_root, crate::log::Role::Standalone, 123);
    assert!(log.starts_with(location.as_path()));
    assert_eq!(std::fs::read_dir(&project).unwrap().count(), 0);
    assert!(
        StateLocation::in_tree(&project, Path::new(".runyte"))
            .plugin_anchor(None)
            .is_none()
    );
    assert_eq!(
        location.plugin_anchor(Some(fixture.path().to_owned())),
        Some(fixture.path().to_owned())
    );
}

#[test]
fn reservations_protect_state_root_and_preserve_windows_cache_sibling() {
    let fixture = TestRuntimeRoot::new("state-res").unwrap();
    let root = fixture.join("local/runyte");
    let project = fixture.join("project");
    let out = StateLocation::out_of_tree(&project, &root);
    out.validate_with_platform(
        &[root.join("cache"), fixture.join("config")],
        None,
        StatePlatform::Windows,
    )
    .unwrap();
    for platform in [StatePlatform::Unix, StatePlatform::MacOs] {
        assert!(
            out.validate_with_platform(&[root.join("cache")], None, platform)
                .is_err()
        );
    }
    assert!(
        out.validate_with_user_root(&[root.join("workspaces/config")], None)
            .is_err()
    );
    assert!(
        StateLocation::InTree(root.join("scratch"))
            .validate_with_user_root(&[], Some(&root))
            .is_err()
    );
    assert!(
        StateLocation::InTree(fixture.path().to_owned())
            .validate_with_user_root(&[], Some(&root))
            .is_err()
    );
    assert!(
        StateLocation::InTree(project.join(".runyte"))
            .validate_with_user_root(&[], Some(&root))
            .is_ok()
    );
}

#[cfg(unix)]
#[test]
fn unsafe_state_roots_and_aliases_are_refused_without_chmod_or_project_artifacts() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = TestRuntimeRoot::new("state-bad").unwrap();
    let root = fixture.create_private_dir("state").unwrap();
    let project = fixture.create_private_dir("project").unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap();
    let location = StateLocation::out_of_tree(&project, &root);
    assert!(location.prepare(&[]).is_err());
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o777
    );
    assert!(!root.join("workspaces").exists());
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&root, fixture.join("alias")).unwrap();
    assert!(
        StateLocation::out_of_tree(&project, &fixture.join("alias"))
            .prepare(&[])
            .is_err()
    );
    assert!(
        StateLocation::InTree(fixture.join("alias/future"))
            .validate_with_user_root(&[], Some(&root))
            .is_err()
    );
    // Foreign-owned existing directories cannot be changed by this process.
    if unsafe { libc::geteuid() } != 0 {
        assert!(
            StateLocation::out_of_tree(&project, Path::new("/"))
                .prepare(&[])
                .is_err()
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_out_of_tree_state_uses_captured_durable_anchor_and_keeps_names_in_workspace() {
    use crate::{
        plugin::state::{Control, Document, Task, run_with_anchor},
        workspace::windows_endpoint::{NameStore, RegistrySet},
    };
    let fixture = TestRuntimeRoot::new("out-state-win").unwrap();
    let project = fixture.create_private_dir("project").unwrap();
    let local = fixture.create_private_dir("local").unwrap();
    let user_root = local.join("runyte");
    let location = StateLocation::out_of_tree(&project, &user_root);
    location.prepare(&[user_root.join("cache")]).unwrap();
    let anchor = location.plugin_anchor(None).unwrap();
    assert_eq!(anchor, local);
    let info = run_with_anchor(
        location.as_path(),
        Some(&anchor),
        "fixture",
        Task::Set {
            expected_revision: "s:missing".into(),
            document: Document {
                version: 1,
                data: serde_json::value::RawValue::from_string("{}".into()).unwrap(),
            },
        },
        &Control::new(),
    )
    .unwrap();
    assert_ne!(info.revision, "s:missing");
    let wrong_anchor = fixture.create_private_dir("foreign-anchor").unwrap();
    assert!(
        run_with_anchor(
            location.as_path(),
            Some(&wrong_anchor),
            "fixture",
            Task::Get,
            &Control::new()
        )
        .is_err()
    );
    let names = NameStore::open(location.as_path()).unwrap();
    let registries = RegistrySet::open(&[fixture.join("registry")]).unwrap();
    let id = crate::workspace::workspace_id(&project);
    assert_eq!(
        names
            .store_if_absent(&registries, &id, "fixture-name")
            .unwrap(),
        "fixture-name"
    );
    assert_eq!(
        NameStore::read_existing(location.as_path(), &id)
            .unwrap()
            .as_deref(),
        Some("fixture-name")
    );
    assert!(location.as_path().join("host-names").is_dir());
    assert!(!project.join(".runyte").exists());
}

#[test]
fn captured_reservations_and_selected_workspace_admission_are_reused_without_creation() {
    use std::{cell::Cell, sync::atomic::Ordering};
    let fixture = TestRuntimeRoot::new("state-once").unwrap();
    let project = fixture.create_private_dir("project").unwrap();
    let config = fixture.create_private_dir("config").unwrap();
    let user_root = fixture.join("not-created/runyte");
    let calls = Cell::new(0);
    let safety = StartupSafety::capture_with(&[config], || {
        calls.set(calls.get() + 1);
        Some(user_root.clone())
    });
    assert_eq!(calls.get(), 1);
    assert_eq!(safety.normalizations.load(Ordering::Relaxed), 2);
    let location = StateLocation::in_tree(&project, Path::new(".runyte"));
    safety.validate(&project, &location).unwrap();
    assert_eq!(safety.normalizations.load(Ordering::Relaxed), 3);
    safety.validate(&project, &location).unwrap();
    safety.validate(&project, &location).unwrap();
    assert_eq!(safety.normalizations.load(Ordering::Relaxed), 3);
    // Even an absolute state path shared by legacy configuration cannot make
    // one workspace's admission authorize a different project identity.
    safety
        .validate(&fixture.join("other-project"), &location)
        .unwrap();
    assert_eq!(safety.normalizations.load(Ordering::Relaxed), 4);
    assert_eq!(calls.get(), 1);
    assert!(!user_root.exists());
    safety.close();
    assert!(safety.validate(&project, &location).is_err());
}

#[cfg(unix)]
#[test]
fn app_consumes_shared_startup_admission_and_cloned_config_rechecks_changed_aliases() {
    use std::{
        os::unix::fs::symlink,
        sync::{Arc, atomic::Ordering},
    };
    let fixture = TestRuntimeRoot::new("state-fresh").unwrap();
    let project = fixture.create_private_dir("project").unwrap();
    let initial = fixture.create_private_dir("reserved").unwrap();
    let alias = fixture.join("alias");
    symlink(&initial, &alias).unwrap();
    let safety = Arc::new(StartupSafety::capture_with(
        std::slice::from_ref(&alias),
        || None,
    ));
    let location = StateLocation::in_tree(&project, Path::new(".runyte"));
    safety.validate(&project, &location).unwrap();
    let checks = safety.normalizations.load(Ordering::Relaxed);
    let mut config = crate::config::Config::default();
    config.lsp.enable = false;
    config.startup_safety = Some(safety.clone());
    let cloned = config.clone();
    let app = crate::app::App::new_in_project(config, None, &project).unwrap();
    assert!(app.config.startup_safety.is_none());
    assert_eq!(safety.normalizations.load(Ordering::Relaxed), checks);
    assert!(safety.admission.lock().unwrap().closed);
    let yaml = app.config.to_yaml().unwrap();
    assert!(!yaml.contains("startup_safety"));
    let reloaded: crate::config::Config = serde_yaml::from_str(&yaml).unwrap();
    assert!(reloaded.startup_safety.is_none());
    std::fs::remove_file(&alias).unwrap();
    symlink(&project, &alias).unwrap();
    // The clone still carries the old Arc, but App must recapture its raw
    // reservations because the first startup consumed that admission.
    let error = crate::app::App::new_in_project(cloned, None, &project)
        .err()
        .unwrap();
    assert!(error.to_string().contains("overlaps per-user"), "{error:#}");
}

#[cfg(unix)]
#[test]
fn failed_app_construction_closes_admission_before_theme_or_project_resolution_errors() {
    use std::{os::unix::fs::symlink, sync::Arc};
    for invalid_theme in [true, false] {
        let fixture = TestRuntimeRoot::new("state-failed").unwrap();
        let project = fixture.create_private_dir("project").unwrap();
        let initial = fixture.create_private_dir("reserved").unwrap();
        let alias = fixture.join("alias");
        symlink(&initial, &alias).unwrap();
        let safety = Arc::new(StartupSafety::capture_with(
            std::slice::from_ref(&alias),
            || None,
        ));
        let location = StateLocation::in_tree(&project, Path::new(".runyte"));
        safety.validate(&project, &location).unwrap();
        let mut config = crate::config::Config::default();
        config.lsp.enable = false;
        config.startup_safety = Some(safety.clone());
        let cloned = config.clone();
        let requested = if invalid_theme {
            config.themes.clear();
            project.clone()
        } else {
            fixture.join("unavailable-project")
        };
        let error = crate::app::App::new_in_project(config, None, requested)
            .err()
            .unwrap();
        if invalid_theme {
            assert!(error.to_string().contains("unknown theme"), "{error:#}");
        }
        assert!(safety.admission.lock().unwrap().closed);
        std::fs::remove_file(&alias).unwrap();
        symlink(&project, &alias).unwrap();
        let error = crate::app::App::new_in_project(cloned, None, &project)
            .err()
            .unwrap();
        assert!(error.to_string().contains("overlaps per-user"), "{error:#}");
    }
}
