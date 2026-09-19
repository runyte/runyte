// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn shared_range_vectors_cover_release_boundaries_and_containment() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/plugins/compatibility/ranges.json"
    ))
    .unwrap();
    for case in vectors["ranges"].as_array().unwrap() {
        let spelling = case["range"].as_str().unwrap();
        let parsed = ReleaseRange::parse(spelling);
        assert_eq!(
            parsed.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{spelling:?}"
        );
        if let Ok(range) = parsed {
            for (key, expected) in [("accept", true), ("reject", false)] {
                for version in case[key].as_array().unwrap() {
                    let version = version.as_str().unwrap();
                    assert_eq!(
                        range.contains(&Version::parse(version).unwrap()),
                        expected,
                        "{spelling:?}: {version}"
                    );
                }
            }
            if let Some(normalized) = case["normalized"].as_str() {
                assert_eq!(range.normalized(), normalized);
            }
        }
    }
    for case in vectors["subsets"].as_array().unwrap() {
        let configured = ReleaseRange::parse(case["configured"].as_str().unwrap()).unwrap();
        let authored = ReleaseRange::parse(case["authored"].as_str().unwrap()).unwrap();
        assert_eq!(
            configured.is_subset_of(&authored),
            case["expected"].as_bool().unwrap(),
            "{case}"
        );
    }
    for version in vectors["invalid_versions"].as_array().unwrap() {
        assert!(
            Version::parse(version.as_str().unwrap()).is_err(),
            "{version}"
        );
    }
}

#[test]
fn optional_features_are_selected_only_when_supported() {
    let names = |values: &[&str]| values.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        negotiate_features(
            &names(&["base"]),
            &names(&["new", "future"]),
            &["base", "new"]
        )
        .unwrap(),
        names(&["base", "new"])
    );
    assert!(negotiate_features(&names(&["missing"]), &names(&[]), &[]).is_err());
    assert!(negotiate_features(&names(&["same"]), &names(&["same"]), &["same"]).is_err());
    for invalid in ["", "white space", "é", &"x".repeat(65)] {
        assert!(negotiate_features(&names(&[]), &names(&[invalid]), &[]).is_err());
    }
    let excessive = (0..33).map(|i| format!("feature-{i}")).collect();
    assert!(negotiate_features(&excessive, &BTreeSet::new(), &[]).is_err());
}

#[test]
fn row_actions_are_advertised_but_never_selected_for_unchanged_plugins() {
    let empty = BTreeSet::new();
    assert!(
        negotiate_features(&empty, &empty, crate::plugin::application::FEATURES)
            .unwrap()
            .is_empty()
    );
    let requested = [crate::plugin::application::VIEW_ROW_ACTIONS.to_owned()].into();
    assert_eq!(
        negotiate_features(&empty, &requested, crate::plugin::application::FEATURES).unwrap(),
        requested
    );
    assert!(
        negotiate_features(&empty, &requested, &[])
            .unwrap()
            .is_empty()
    );
}
