// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use std::{fs, os::windows::ffi::OsStringExt};

fn inputs(root: &TestRuntimeRoot, roots: CapturedRoots) -> LocationInputs {
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    LocationInputs {
        state_root: project.join(".runyte"),
        project_root: project,
        reserved_user_roots: vec![root.join("config")],
        roots,
    }
}

#[test]
fn parent_initialization_respects_removal_lease_and_child_root_identity() {
    let root = TestRuntimeRoot::new("location-init-lease").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let inventory = root.join("inventory");
    let scope = DiscoveryScope::resolve(DiscoveryInputs {
        reserved_user_roots: vec![root.join("config")],
        roots: CapturedRoots {
            runtime_root: Some(root.create_private_dir("runtime").unwrap()),
            inventory_override: Some(inventory.clone()),
            ..CapturedRoots::default()
        },
    })
    .unwrap();
    let removal = ProjectLease::acquire(&project, &inventory).unwrap();
    let state = project.join(".runyte");
    let error = scope
        .initialize_layout(&project, Path::new(".runyte"))
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    assert!(!state.exists());
    drop(removal);

    let parent = scope
        .initialize_layout(&project, Path::new(".runyte"))
        .unwrap();
    assert!(state.is_dir());
    let expected = parent.fingerprint().unwrap();
    fs::rename(&project, root.join("moved-project")).unwrap();
    fs::create_dir(&project).unwrap();
    assert_eq!(
        parent.acquire_project_lease().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let child = ResolvedLayout::from_scope(scope, &project, state).unwrap();
    assert!(
        child
            .verify_detached_layout(true, Some(OsStr::new(&expected)))
            .is_err()
    );
}

#[test]
fn runtime_and_cache_select_shared_primary_and_runtime_secondary_without_writes() {
    let root = TestRuntimeRoot::new("location-shared").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    let cache_home = root.join("cache-home");
    let layout = ResolvedLayout::resolve(inputs(
        &root,
        CapturedRoots {
            runtime_root: Some(runtime.clone()),
            cache_home: Some(cache_home.clone()),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
    ))
    .unwrap();
    assert_eq!(
        layout.endpoint_directory(),
        runtime
            .join("runyte")
            .join(super::super::workspace_id(layout.project_root()))
    );
    assert_eq!(
        layout.namespace_roots(),
        [
            cache_home.join("runyte/hosts"),
            runtime.join("runyte/hosts")
        ]
    );
    assert_eq!(
        layout.name_store_root(),
        layout.state_root().join("host-names")
    );
    assert!(
        layout
            .discovery_view(false)
            .unwrap()
            .scan_namespaces()
            .candidates
            .is_empty()
    );
    assert!(!cache_home.exists());
    assert!(!runtime.join("runyte").exists());
    assert!(!layout.state_root().exists());
    assert!(!root.join("inventory").exists());
}

#[test]
fn runtime_only_cache_only_and_no_namespace_fallbacks_are_explicit() {
    let root = TestRuntimeRoot::new("location-fallbacks").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    let layout = ResolvedLayout::resolve(inputs(
        &root,
        CapturedRoots {
            runtime_root: Some(runtime.clone()),
            ..CapturedRoots::default()
        },
    ))
    .unwrap();
    assert_eq!(layout.namespace_roots(), [runtime.join("runyte/hosts")]);
    assert!(layout.discovery_view(false).is_ok());
    // Missing account inventory is irrelevant to ordinary observation.
    assert!(layout.discovery_view(true).is_err());
    assert!(layout.publication_location().is_err());
    let cache_home = root.join("cache-home");
    let layout = ResolvedLayout::resolve(inputs(
        &root,
        CapturedRoots {
            cache_home: Some(cache_home.clone()),
            ..CapturedRoots::default()
        },
    ))
    .unwrap();
    assert_eq!(
        layout.endpoint_directory(),
        layout.state_root().join("host")
    );
    assert_eq!(layout.namespace_roots(), [cache_home.join("runyte/hosts")]);
    let layout = ResolvedLayout::resolve(inputs(&root, CapturedRoots::default())).unwrap();
    assert!(layout.namespace_roots().is_empty());
    assert!(layout.discovery_view(false).is_ok());
    assert!(layout.publication_location().is_err());
    assert!(!layout.state_root().exists());
}

#[test]
fn invalid_runtime_is_ignored_and_removed_from_explicit_child_environment() {
    let root = TestRuntimeRoot::new("location-runtime").unwrap();
    let runtime = root.join("ordinary-runtime");
    fs::create_dir(&runtime).unwrap();
    assert!(Directory::open_existing(&runtime, true).is_err());
    let layout = ResolvedLayout::resolve(inputs(
        &root,
        CapturedRoots {
            runtime_root: Some(runtime.clone()),
            cache_home: Some(root.join("cache-home")),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
    ))
    .unwrap();
    assert_eq!(
        layout.endpoint_directory(),
        layout.state_root().join("host")
    );
    assert!(
        layout
            .detached_environment()
            .unwrap()
            .contains(&("XDG_RUNTIME_DIR".into(), None))
    );
    assert!(Directory::open_existing(&runtime, true).is_err());
}

#[test]
fn unusable_cache_allows_runtime_publication_but_not_a_complete_discovery_scan() {
    let root = TestRuntimeRoot::new("location-cache-error").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    let cache_home = root.join("file");
    fs::write(&cache_home, b"unchanged").unwrap();
    let layout = ResolvedLayout::resolve(inputs(
        &root,
        CapturedRoots {
            runtime_root: Some(runtime.clone()),
            cache_home: Some(cache_home.clone()),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
    ))
    .unwrap();
    assert_eq!(layout.namespace_roots(), [runtime.join("runyte/hosts")]);
    assert!(layout.discovery_view(false).is_err());
    layout.publication_location().unwrap();
    assert_eq!(fs::read(cache_home).unwrap(), b"unchanged");
}

#[test]
fn nonlocal_optional_cache_is_refused_before_io_without_disabling_runtime_publication() {
    let root = TestRuntimeRoot::new("location-unc-cache").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    let layout = ResolvedLayout::resolve(inputs(
        &root,
        CapturedRoots {
            runtime_root: Some(runtime.clone()),
            cache_home: Some(PathBuf::from(r"\\example.invalid\cache")),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
    ))
    .unwrap();
    assert_eq!(layout.namespace_roots(), [runtime.join("runyte/hosts")]);
    assert!(layout.discovery_view(false).is_err());
    assert!(!runtime.join("runyte").exists());
    assert!(!root.join("inventory").exists());
    layout.publication_location().unwrap();
    assert!(runtime.join("runyte/hosts/.registry.lock").exists());
    assert!(root.join("inventory/.registry.lock").exists());
}

#[test]
fn inventory_overlap_is_checked_before_publication_mutates_any_root() {
    let root = TestRuntimeRoot::new("location-inventory-overlap").unwrap();
    let cache_home = root.join("cache-home");
    let inventory = root.join("project/.runyte/inventory");
    let layout = ResolvedLayout::resolve(inputs(
        &root,
        CapturedRoots {
            cache_home: Some(cache_home.clone()),
            inventory_override: Some(inventory),
            ..CapturedRoots::default()
        },
    ))
    .unwrap();
    assert!(layout.discovery_view(false).is_ok());
    assert!(layout.publication_location().is_err());
    assert!(!cache_home.exists());
    assert!(!layout.state_root().exists());
}

#[test]
fn injected_os_defaults_match_cache_and_inventory_contract_without_account_access() {
    let root = TestRuntimeRoot::new("location-defaults").unwrap();
    let account = root.join("account");
    let roots = CapturedRoots {
        local_app_data: Some(account.clone()),
        ..CapturedRoots::default()
    };
    assert_eq!(roots.cache_root(), Some(account.join("runyte/cache")));
    assert_eq!(
        roots.inventory_root().unwrap(),
        account.join("runyte/all-hosts")
    );
    let layout = ResolvedLayout::resolve(inputs(&root, roots)).unwrap();
    assert_eq!(
        layout.namespace_roots(),
        [account.join("runyte/cache/hosts")]
    );
    assert!(!account.exists());
    let environment = layout.detached_environment().unwrap();
    assert!(environment.contains(&("XDG_CACHE_HOME".into(), None)));
    assert!(environment.contains(&(
        "RUNYTE_ALL_HOSTS_DIR".into(),
        Some(account.join("runyte/all-hosts").into_os_string())
    )));
}

#[test]
fn explicit_invalid_inventory_does_not_fall_back_to_the_account() {
    let root = TestRuntimeRoot::new("location-inventory").unwrap();
    let layout = ResolvedLayout::resolve(inputs(
        &root,
        CapturedRoots {
            cache_home: Some(root.join("cache-home")),
            local_app_data: Some(root.join("account")),
            inventory_override: Some("relative".into()),
            ..CapturedRoots::default()
        },
    ))
    .unwrap();
    assert!(layout.discovery_view(false).is_ok());
    assert!(layout.discovery_view(true).is_err());
    assert!(layout.publication_location().is_err());
    assert!(!root.join("account").exists());
    assert!(!root.join("cache-home").exists());
}

#[test]
fn state_overlap_and_nonlocal_or_unbounded_locations_are_refused() {
    let root = TestRuntimeRoot::new("location-validation").unwrap();
    let mut value = inputs(&root, CapturedRoots::default());
    value.state_root = root.join("config/project-state");
    assert!(ResolvedLayout::resolve(value).is_err());
    for path in [
        PathBuf::from("relative"),
        PathBuf::from(r"\\server\share\root"),
        PathBuf::from("C:\\nul\0unit"),
        PathBuf::from(format!("C:\\{}", "x".repeat(MAX_PERSISTED_PATH_BYTES))),
    ] {
        assert!(validate_path(&path).is_err());
    }
}

#[test]
fn missing_state_ancestors_cannot_hide_case_or_verbatim_reserved_root_aliases() {
    let root = TestRuntimeRoot::new("location-case-overlap").unwrap();
    let mut value = inputs(&root, CapturedRoots::default());
    value.state_root = root.join("CONFIG/missing/project-state");
    value.reserved_user_roots = vec![root.join("config/missing")];
    assert!(ResolvedLayout::resolve(value).is_err());
    assert!(!root.join("config").exists());

    let units: Vec<_> = root.as_os_str().encode_wide().collect();
    assert!(units.starts_with(&[92, 92, 63, 92]));
    let ordinary = PathBuf::from(OsString::from_wide(&units[4..]));
    let mut value = inputs(&root, CapturedRoots::default());
    value.state_root = ordinary.join("config/missing/project-state");
    value.reserved_user_roots = vec![root.join("CONFIG/MISSING")];
    assert!(ResolvedLayout::resolve(value).is_err());
    assert!(!root.join("config").exists());

    // Overlap is bidirectional and compares whole components, not prefixes.
    assert!(
        validate_state_separation(&ordinary.join("STATE"), &[root.join("state/nested/cache")])
            .is_err()
    );
    validate_state_separation(
        &ordinary.join("state-a/nested"),
        &[root.join("state-b/nested")],
    )
    .unwrap();
    assert!(!root.join("state").exists());
    assert!(!root.join("state-a").exists());
    assert!(!root.join("state-b").exists());
}

#[test]
fn fingerprint_preserves_native_units_field_framing_and_namespace_order() {
    let mut first = Vec::new();
    let mut second = Vec::new();
    let lone = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd800]));
    let replacement = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xfffd]));
    fingerprint_path(&mut first, 1, &lone).unwrap();
    fingerprint_path(&mut second, 1, &replacement).unwrap();
    assert_ne!(first, second);
    assert_eq!(&first[5..], encode_path(&lone));
    second.clear();
    fingerprint_path(&mut second, 2, &lone).unwrap();
    assert_ne!(first, second);
    let root = TestRuntimeRoot::new("location-fingerprint").unwrap();
    let mut layout = ResolvedLayout::resolve(inputs(
        &root,
        CapturedRoots {
            runtime_root: Some(root.create_private_dir("runtime").unwrap()),
            cache_home: Some(root.join("cache-home")),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
    ))
    .unwrap();
    let original = layout.fingerprint().unwrap();
    layout.scope.namespaces.reverse();
    assert_ne!(original, layout.fingerprint().unwrap());
    let mut joined_a = Vec::new();
    fingerprint_path(&mut joined_a, 1, Path::new(r"C:\a")).unwrap();
    fingerprint_path(&mut joined_a, 1, Path::new(r"C:\bc")).unwrap();
    let mut joined_b = Vec::new();
    fingerprint_path(&mut joined_b, 1, Path::new(r"C:\ab")).unwrap();
    fingerprint_path(&mut joined_b, 1, Path::new(r"C:\c")).unwrap();
    assert_ne!(joined_a, joined_b);
}

#[test]
fn changed_parent_fallback_is_rejected_only_for_a_dedicated_detached_launch() {
    let root = TestRuntimeRoot::new("location-child").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    let roots = CapturedRoots {
        runtime_root: Some(runtime.clone()),
        cache_home: Some(root.join("cache-home")),
        inventory_override: Some(root.join("inventory")),
        ..CapturedRoots::default()
    };
    let parent = ResolvedLayout::resolve(inputs(&root, roots.clone())).unwrap();
    let expected = OsString::from(parent.fingerprint().unwrap());
    parent
        .verify_detached_layout(true, Some(&expected))
        .unwrap();
    fs::rename(runtime, root.join("moved-runtime")).unwrap();
    let child = ResolvedLayout::resolve(inputs(&root, roots)).unwrap();
    assert_ne!(parent.endpoint_directory(), child.endpoint_directory());
    assert!(child.verify_detached_layout(true, Some(&expected)).is_err());
    child
        .verify_detached_layout(false, Some(&expected))
        .unwrap();
    child.verify_detached_layout(true, None).unwrap();
    let marker = child
        .detached_environment()
        .unwrap()
        .into_iter()
        .find(|(key, _)| key == EXPECTED_LAYOUT_ENV)
        .unwrap()
        .1
        .unwrap();
    assert_ne!(marker, expected);
    assert!(!root.join("inventory").exists());
}

#[test]
fn projectless_scope_and_direct_layout_preserve_fingerprint_and_environment() {
    let root = TestRuntimeRoot::new("scope-layout-parity").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    for runtime_root in [None, Some(runtime)] {
        for cache_home in [None, Some(root.join("cache-home"))] {
            let roots = CapturedRoots {
                runtime_root: runtime_root.clone(),
                cache_home,
                local_app_data: Some(root.join("account")),
                inventory_override: Some(root.join("inventory")),
            };
            let scope = DiscoveryScope::resolve(DiscoveryInputs {
                roots: roots.clone(),
                reserved_user_roots: vec![root.join("config")],
            })
            .unwrap();
            let direct = ResolvedLayout::resolve(inputs(&root, roots)).unwrap();
            let composed = ResolvedLayout::from_scope(
                scope,
                direct.project_root(),
                direct.state_root().to_owned(),
            )
            .unwrap();
            assert_eq!(composed.namespace_roots(), direct.namespace_roots());
            assert_eq!(composed.cache_root().unwrap(), direct.cache_root().unwrap());
            assert_eq!(composed.endpoint_directory(), direct.endpoint_directory());
            assert_eq!(composed.name_store_root(), direct.name_store_root());
            assert_eq!(
                composed.fingerprint().unwrap(),
                direct.fingerprint().unwrap()
            );
            assert_eq!(
                composed.detached_environment().unwrap(),
                direct.detached_environment().unwrap()
            );
        }
    }
    assert!(!root.join("cache-home").exists());
    assert!(!root.join("account").exists());
    assert!(!root.join("inventory").exists());
    assert!(!root.join("project/.runyte").exists());
}

#[test]
fn scope_freezes_missing_runtime_and_reserved_paths_without_a_project() {
    let root = TestRuntimeRoot::new("scope-frozen").unwrap();
    let runtime = root.join("runtime");
    let scope = DiscoveryScope::resolve(DiscoveryInputs {
        roots: CapturedRoots {
            runtime_root: Some(runtime.clone()),
            cache_home: Some(root.join("cache")),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
        reserved_user_roots: vec![root.join("reserved")],
    })
    .unwrap();
    assert_eq!(scope.namespace_roots(), &[root.join("cache/runyte/hosts")]);
    assert!(!root.join("project").exists());
    root.create_private_dir("runtime").unwrap();
    let absent = root.join("missing-project");
    let known = scope
        .known_read_location(&absent, &absent.join(".runyte"))
        .unwrap();
    assert_eq!(known.endpoint_directory(), absent.join(".runyte/host"));
    assert!(
        scope
            .known_read_location(&absent, &root.join("reserved/state"))
            .is_err()
    );
    let project = root.create_private_dir("project").unwrap();
    let composed = ResolvedLayout::from_scope(scope, &project, project.join(".runyte")).unwrap();
    assert_eq!(composed.endpoint_directory(), project.join(".runyte/host"));
    assert!(
        composed
            .detached_environment()
            .unwrap()
            .contains(&("XDG_RUNTIME_DIR".into(), None))
    );
    assert!(!root.join("cache").exists());
    assert!(!root.join("reserved").exists());
    assert!(!absent.exists());
}

#[test]
fn scope_selected_runtime_is_not_readmitted_after_it_disappears() {
    let root = TestRuntimeRoot::new("scope-runtime-removal").unwrap();
    let runtime = root.create_private_dir("runtime").unwrap();
    let scope = DiscoveryScope::resolve(DiscoveryInputs {
        roots: CapturedRoots {
            runtime_root: Some(runtime.clone()),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
        reserved_user_roots: vec![],
    })
    .unwrap();
    fs::remove_dir(&runtime).unwrap();
    let project = root.create_private_dir("project").unwrap();
    let layout = ResolvedLayout::from_scope(scope, &project, project.join(".runyte")).unwrap();
    assert_eq!(
        layout.endpoint_directory(),
        runtime
            .join("runyte")
            .join(super::super::workspace_id(layout.project_root()))
    );
    assert!(
        layout
            .detached_environment()
            .unwrap()
            .contains(&("XDG_RUNTIME_DIR".into(), Some(runtime.into_os_string())))
    );
    assert!(!project.join(".runyte").exists());
}
