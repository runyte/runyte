// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;

#[test]
fn home_exception_rejects_project_cache_overrides_and_symlinks() {
    let root = TestRuntimeRoot::new("trust-home").unwrap();
    let home = root.create_private_dir("home").unwrap();
    let project = home.join("project");
    std::fs::create_dir(&project).unwrap();
    let standard = home.join(TrustStore::HOME_CACHE);
    assert!(TrustStore::new_with_home(Some(standard.clone()), &home, Some(&home)).is_ok());
    assert!(TrustStore::new_with_home(Some(standard.clone()), &home, None).is_err());
    assert!(
        TrustStore::new_with_home(Some(home.join("custom-cache")), &home, Some(&home)).is_err()
    );
    assert!(
        TrustStore::new_with_home(
            Some(project.join(TrustStore::HOME_CACHE)),
            &project,
            Some(&home)
        )
        .is_err()
    );
    assert!(TrustStore::new_with_home(Some(standard.clone()), &root, Some(&home)).is_err());
    assert!(TrustStore::new_with_home(Some(standard.clone()), &project, Some(&home)).is_ok());

    let alias = root.join("home-alias");
    std::os::unix::fs::symlink(&home, &alias).unwrap();
    let store = TrustStore::new_with_home(Some(standard.clone()), &alias, Some(&home)).unwrap();
    store.save(true).unwrap();
    assert_eq!(store.load().unwrap(), Some(true));

    // Recognizing the standard location must not bypass secure opening.
    std::fs::remove_dir_all(&standard).unwrap();
    let outside = root.create_private_dir("outside").unwrap();
    std::os::unix::fs::symlink(&outside, &standard).unwrap();
    assert!(store.load().is_err());
    assert!(store.save(true).is_err());
    assert!(std::fs::read_dir(outside).unwrap().next().is_none());
}
