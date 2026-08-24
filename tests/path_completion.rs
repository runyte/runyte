// SPDX-License-Identifier: MPL-2.0

//! Path completion over directory trees large enough to need a bound.
//!
//! Both path popups — the one insert mode opens on `/` and the command
//! palette's rows for a path argument — keep at most a few hundred entries so
//! that filesystem work on the input thread stays bounded. What matters is
//! *which* entries survive that bound. A directory read returns names in
//! whatever order the filesystem holds them, which for a large directory is
//! neither sorted nor stable, so a bound applied before the typed prefix is
//! consulted keeps an arbitrary slice and hides every match outside it. These
//! tests type names that exist and insist they are offered, at sizes where an
//! unfiltered bound could only find them by luck.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use runyte::{
    app::{App, CompletionSource},
    config::Config,
    input::{KeyCode, KeyStroke, Modifiers},
};

/// Entries per directory in the wide trees below.
///
/// Comfortably past the 512-entry popup bound, so a listing cut before
/// filtering keeps well under a tenth of the directory: ten probes each
/// naming an entry that exists cannot all be found in that slice by accident.
const WIDE: usize = 6_000;

fn temporary(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "runyte-path-completion-{}-{nanos}-{name}",
        std::process::id()
    ))
}

/// A directory holding `WIDE` files, `WIDE / 10` subdirectories, and a few
/// hidden entries, with names spread across the alphabet so no prefix is a
/// prefix of everything.
fn wide_directory(path: &Path) {
    fs::create_dir_all(path).unwrap();
    for index in 0..WIDE {
        fs::write(path.join(format!("file_{index:05}.txt")), "").unwrap();
        if index % 10 == 0 {
            fs::create_dir_all(path.join(format!("dir_{index:05}"))).unwrap();
        }
    }
    for index in 0..8 {
        fs::write(path.join(format!(".hidden_{index}")), "").unwrap();
    }
}

/// A chain of `depth` nested directories, each also holding `WIDE / 20`
/// files, so descending it crosses a large listing at every step.
fn deep_directory(root: &Path, depth: usize) -> PathBuf {
    let mut path = root.to_path_buf();
    for level in 0..depth {
        path = path.join(format!("level_{level:02}"));
        fs::create_dir_all(&path).unwrap();
        for index in 0..WIDE / 20 {
            fs::write(path.join(format!("noise_{index:05}.txt")), "").unwrap();
        }
    }
    path
}

fn press(app: &mut App, character: char) {
    app.handle_key(KeyStroke::new(KeyCode::Char(character), Modifiers::NONE))
        .unwrap();
}

fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        press(app, character);
    }
}

/// An editor whose active buffer is an empty note directly under `root`.
fn editor(root: &Path) -> App {
    let active = root.join("note.txt");
    if !active.exists() {
        fs::write(&active, "").unwrap();
    }
    App::new_in_project(Config::default(), Some(active.clone()), root).unwrap()
}

/// The labels the insert-mode popup offers after typing `path`, filtered the
/// way the popup itself filters when it draws.
fn insert_completions(root: &Path, path: &str) -> Vec<String> {
    let mut app = editor(root);
    press(&mut app, 'i');
    type_text(&mut app, path);
    let Some(state) = app.completion.as_ref() else {
        return Vec::new();
    };
    assert_eq!(state.source, CompletionSource::Path);
    state
        .visible_indices()
        .into_iter()
        .map(|index| state.items[index].label.clone())
        .collect()
}

/// The palette rows offered after typing `:open <argument>`.
fn palette_hints(root: &Path, argument: &str) -> Vec<String> {
    let mut app = editor(root);
    press(&mut app, ':');
    type_text(&mut app, &format!("open {argument}"));
    app.matching_path_hints()
        .expect("a path argument owns the rows")
        .into_iter()
        .map(|hint| hint.value)
        .collect()
}

#[test]
fn a_wide_directory_offers_every_name_typed_into_it() {
    let root = temporary("wide-insert");
    let wide = root.join("wide");
    wide_directory(&wide);

    // The popup that opens on the slash is bounded, and says so by holding
    // exactly the bound rather than the directory.
    let opened = insert_completions(&root, "wide/");
    assert_eq!(opened.len(), 512);

    // Every probe names a file that exists. Before the prefix reached the
    // listing, the bound kept an arbitrary slice of the directory and nine of
    // these ten found nothing.
    for probe in [0, 137, 2_500, 4_999, 5_123, 5_777, 5_900, 5_998, 5_999, 42] {
        let typed = format!("wide/file_{probe:05}.");
        let offered = insert_completions(&root, &typed);
        assert_eq!(
            offered,
            vec![format!("file_{probe:05}.txt")],
            "typing {typed} should offer the file it names"
        );
    }

    // Subdirectories are reached the same way, and keep the trailing
    // separator that lets the next keystroke continue into them.
    let offered = insert_completions(&root, "wide/dir_05990");
    assert_eq!(offered, vec!["dir_05990/".to_owned()]);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_wide_directory_offers_every_name_typed_into_the_palette() {
    let root = temporary("wide-palette");
    let wide = root.join("wide");
    wide_directory(&wide);
    let base = wide.display().to_string();

    let opened = palette_hints(&root, &format!("{base}/"));
    assert_eq!(opened.len(), 512);
    // Directories sort before plain files, so an unbounded listing and a
    // bounded one still agree about what the first row is.
    assert!(opened[0].ends_with("dir_00000/"));

    for probe in [0, 137, 2_500, 4_999, 5_123, 5_777, 5_900, 5_998, 5_999, 42] {
        let typed = format!("{base}/file_{probe:05}.");
        let offered = palette_hints(&root, &typed);
        assert_eq!(
            offered,
            vec![format!("{base}/file_{probe:05}.txt")],
            "typing {typed} should offer the file it names"
        );
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn typing_further_into_a_wide_directory_narrows_rather_than_empties() {
    let root = temporary("wide-narrowing");
    let wide = root.join("wide");
    wide_directory(&wide);

    // Each keystroke of a name that exists has to leave the popup open and
    // non-empty. The bug this covers emptied it one character after the
    // slash and left it empty until the next slash.
    //
    // While more than the bound still matches, the popup holds the smallest
    // matching names rather than the one being typed — that is what a bound
    // means. What it may not do is lose the name once the typed prefix
    // narrows the directory to fewer entries than the bound.
    let mut app = editor(&root);
    press(&mut app, 'i');
    type_text(&mut app, "wide/");
    for character in "file_05999.txt".chars() {
        press(&mut app, character);
        let state = app
            .completion
            .as_ref()
            .unwrap_or_else(|| panic!("the popup closed while typing at {character}"));
        let labels = state
            .visible_indices()
            .into_iter()
            .map(|index| state.items[index].label.clone())
            .collect::<Vec<_>>();
        assert!(!labels.is_empty(), "the popup emptied at {character}");
        if labels.len() < 512 {
            assert!(
                labels.iter().any(|label| label.starts_with("file_05999")),
                "the popup stopped offering file_05999.txt at {character}"
            );
        }
    }
    let state = app.completion.as_ref().expect("the popup stays open");
    assert_eq!(
        state
            .visible_indices()
            .into_iter()
            .map(|index| state.items[index].label.clone())
            .collect::<Vec<_>>(),
        vec!["file_05999.txt".to_owned()]
    );

    // A name that does not exist still closes the popup rather than showing
    // rows that cannot match it.
    press(&mut app, 'x');
    assert!(app.completion.is_none());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_deep_tree_completes_at_every_level() {
    let root = temporary("deep");
    let deepest = deep_directory(&root, 12);
    fs::write(deepest.join("target.txt"), "").unwrap();

    // Each level is typed in turn, from the shallowest to the deepest, and
    // every one of them has to offer the level below it.
    let mut typed = String::new();
    for level in 0..12 {
        typed.push_str(&format!("level_{level:02}/"));
        if level < 11 {
            let offered = insert_completions(&root, &format!("{typed}level_{:02}", level + 1));
            assert_eq!(
                offered,
                vec![format!("level_{:02}/", level + 1)],
                "level {level} should offer the level below it"
            );
        }
    }
    assert_eq!(
        insert_completions(&root, &format!("{typed}target")),
        vec!["target.txt".to_owned()]
    );
    assert_eq!(
        palette_hints(&root, &format!("{}/target", deepest.display())),
        vec![format!("{}/target.txt", deepest.display())]
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hidden_entries_stay_hidden_until_a_dot_is_typed() {
    let root = temporary("hidden");
    let wide = root.join("wide");
    wide_directory(&wide);
    let base = wide.display().to_string();

    // The palette hides dot entries until the argument asks for one, and a
    // directory far larger than the bound must not change that either way.
    let opened = palette_hints(&root, &format!("{base}/"));
    assert!(!opened.iter().any(|value| value.contains("/.hidden_")));
    let offered = palette_hints(&root, &format!("{base}/.hidden_5"));
    assert_eq!(offered, vec![format!("{base}/.hidden_5")]);

    // Insert-mode completion offers dot entries only once the dot is typed,
    // because the typed prefix is what selects them.
    assert_eq!(
        insert_completions(&root, "wide/.hidden_5"),
        vec![".hidden_5".to_owned()]
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn names_that_are_prefixes_of_each_other_all_stay_reachable() {
    let root = temporary("prefixes");
    let nest = root.join("nest");
    fs::create_dir_all(&nest).unwrap();
    // A run of names where each is a prefix of the next, buried in a
    // directory too large to offer whole.
    for index in 0..WIDE {
        fs::write(nest.join(format!("other_{index:05}.txt")), "").unwrap();
    }
    let mut name = String::from("z");
    for _ in 0..12 {
        fs::write(nest.join(format!("{name}.txt")), "").unwrap();
        name.push('z');
    }

    // Each step of the run narrows to exactly the names that extend it.
    let mut typed = String::from("z");
    for step in 0..12 {
        let offered = insert_completions(&root, &format!("nest/{typed}"));
        assert_eq!(
            offered.len(),
            12 - step,
            "typing {typed} should offer every name extending it"
        );
        assert!(offered.contains(&format!("{typed}.txt")));
        typed.push('z');
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_wide_directory_of_unicode_names_completes_on_the_typed_prefix() {
    let root = temporary("unicode");
    let wide = root.join("wide");
    fs::create_dir_all(&wide).unwrap();
    for index in 0..WIDE {
        fs::write(wide.join(format!("файл_{index:05}.txt")), "").unwrap();
    }
    fs::write(wide.join("café_menu.txt"), "").unwrap();

    assert_eq!(
        insert_completions(&root, "wide/café"),
        vec!["café_menu.txt".to_owned()]
    );
    assert_eq!(
        insert_completions(&root, "wide/файл_05999"),
        vec!["файл_05999.txt".to_owned()]
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_symlinked_directory_is_offered_as_a_directory() {
    #[cfg(unix)]
    {
        let root = temporary("symlink");
        let wide = root.join("wide");
        wide_directory(&wide);
        std::os::unix::fs::symlink(&wide, root.join("link_to_wide")).unwrap();

        // The trailing separator is what tells a person the completion can be
        // continued, so following the link to answer it has to survive the
        // cheaper entry-kind check a large listing needs.
        assert_eq!(
            insert_completions(&root, "./link_to_"),
            vec!["link_to_wide/".to_owned()]
        );
        let offered = palette_hints(&root, &format!("{}/link_to_", root.display()));
        assert_eq!(offered, vec![format!("{}/link_to_wide/", root.display())]);

        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn the_bounded_rows_are_the_ones_the_full_listing_would_lead_with() {
    let root = temporary("bounded-order");
    let mixed = root.join("mixed");
    fs::create_dir_all(&mixed).unwrap();
    // Names whose case alternates, so the case-insensitive order the palette
    // shows and the exact order the insert popup shows disagree, and both are
    // deeper than the bound.
    //
    // No two names differ only in case: a filesystem that ignores case could
    // not hold both, and what is under test is the ordering rather than the
    // filesystem's own idea of identity.
    let mut names = Vec::new();
    for index in 0..700 {
        // Few enough directories that the bound falls among the files, which
        // are shown after them.
        if index % 7 == 0 {
            let name = format!("Alpha_{index:04}");
            fs::create_dir_all(mixed.join(&name)).unwrap();
            names.push((name, true));
        }
        for stem in ["beta", "Gamma", "delta", "Epsilon"] {
            let name = format!("{stem}_{index:04}");
            fs::write(mixed.join(&name), "").unwrap();
            names.push((name, false));
        }
    }

    // What the insert popup shows: every name as its label, in the order a
    // sorted listing would put them in, cut at the bound.
    let mut labels = names
        .iter()
        .map(|(name, is_directory)| {
            if *is_directory {
                format!("{name}/")
            } else {
                name.clone()
            }
        })
        .collect::<Vec<_>>();
    labels.sort();
    labels.truncate(512);
    assert_eq!(insert_completions(&root, "mixed/"), labels);

    // What the palette shows: directories first, then without regard to case,
    // then by exact spelling, cut at the same bound.
    let base = mixed.display().to_string();
    let mut rows = names
        .iter()
        .map(|(name, is_directory)| {
            let row = if *is_directory {
                format!("{name}{}", std::path::MAIN_SEPARATOR)
            } else {
                name.clone()
            };
            (!*is_directory, row.to_lowercase(), row)
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows.truncate(512);
    let expected = rows
        .into_iter()
        .map(|(_, _, row)| format!("{base}{}{row}", std::path::MAIN_SEPARATOR))
        .collect::<Vec<_>>();
    assert_eq!(palette_hints(&root, &format!("{base}/")), expected);

    // The same has to hold once a prefix narrows the directory, where the
    // bound falls in a different place.
    let mut narrowed = names
        .iter()
        .filter(|(name, _)| name.starts_with("beta_0"))
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    narrowed.sort();
    narrowed.truncate(512);
    assert_eq!(insert_completions(&root, "mixed/beta_0"), narrowed);

    fs::remove_dir_all(root).unwrap();
}
