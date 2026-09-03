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

use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use runyte::{
    app::{App, CompletionSource},
    config::Config,
    input::{KeyCode, KeyStroke, Modifiers},
    key_hints::KeyHintState,
    ui,
    workspace::WorkspaceHost,
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

/// The rows the finder-path prompt offers after typing `path` into it.
///
/// `Space / p` is the whole prompt, so unlike the palette there is no command
/// name in front of the path and no argument gate to pass.
fn finder_path_hints(root: &Path, path: &str) -> Vec<String> {
    let mut app = editor(root);
    press(&mut app, ' ');
    press(&mut app, '/');
    press(&mut app, 'p');
    type_text(&mut app, path);
    app.finder_path_hints()
        .expect("the finder-path prompt owns the rows")
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

#[test]
fn a_wide_directory_offers_every_name_typed_into_the_finder_path_prompt() {
    let root = temporary("wide-finder-path");
    let wide = root.join("wide");
    wide_directory(&wide);
    let base = wide.display().to_string();

    let opened = finder_path_hints(&root, &format!("{base}/"));
    assert_eq!(opened.len(), 512);
    assert!(opened[0].ends_with("dir_00000/"));

    for probe in [0, 2_500, 5_123, 5_999] {
        let typed = format!("{base}/file_{probe:05}.");
        let offered = finder_path_hints(&root, &typed);
        assert_eq!(
            offered,
            vec![format!("{base}/file_{probe:05}.txt")],
            "typing {typed} should offer the file it names"
        );
    }

    // Tab takes the selected row as the whole prompt, with no quoting to
    // separate it from an argument that is not there.
    let mut app = editor(&root);
    press(&mut app, ' ');
    press(&mut app, '/');
    press(&mut app, 'p');
    type_text(&mut app, &format!("{base}/dir_05990"));
    app.handle_key(KeyStroke::new(KeyCode::Tab, Modifiers::NONE))
        .unwrap();
    assert_eq!(
        app.command,
        format!("{base}/dir_05990{}", std::path::MAIN_SEPARATOR)
    );

    fs::remove_dir_all(root).unwrap();
}

// The assistance a completing prompt draws is one surface with two
// renderings. A standalone editor draws it from live state and an attached
// client draws it from the published snapshot, and the two used to disagree
// about everything but the rows: one anchored a bordered list to the bottom
// left, the other centred a box over most of the editor. These tests hold the
// two readings against each other.

/// A directory with two entries whose names are short enough to read in a
/// rendered frame and distinct enough to tell apart.
fn small_directory(path: &Path) -> PathBuf {
    fs::create_dir_all(path.join("folder")).unwrap();
    fs::write(path.join("file.txt"), "").unwrap();
    path.to_path_buf()
}

fn screen(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|row| {
            (0..buffer.area.width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn standalone_screen(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let prepared = app.prepare_view(ui::frame_geometry(frame.area()));
            let snapshot = app.snapshot(&prepared);
            ui::render_exact_colors_for_test(frame, app, &snapshot, &KeyHintState::default());
        })
        .unwrap();
    screen(terminal.backend().buffer())
}

fn attached_screen(app: App, width: u16, height: u16) -> Vec<String> {
    let mut host = WorkspaceHost::new(app);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let published = host.prepare_frame(ui::frame_geometry(frame.area()));
            ui::render_host_frame_exact_colors_for_test(frame, &published);
        })
        .unwrap();
    screen(terminal.backend().buffer())
}

/// Where the bordered assistance sits and what it holds: the row it starts
/// on, the column its left border is in, its width, and its rows including
/// both borders.
fn hint_box(screen: &[String]) -> (usize, usize, usize, Vec<String>) {
    let top = screen
        .iter()
        .position(|line| line.contains("Choose path for"))
        .expect("the assistance names itself");
    let left = screen[top]
        .chars()
        .position(|character| character == '┌')
        .expect("a left border");
    let width = screen[top]
        .chars()
        .skip(left)
        .position(|character| character == '┐')
        .map(|index| index + 1)
        .expect("a right border");
    let rows = screen[top..]
        .iter()
        .map(|line| line.chars().skip(left).take(width).collect::<String>())
        .take_while(|row| row.starts_with('┌') || row.starts_with('│') || row.starts_with('└'))
        .collect::<Vec<_>>();
    (top, left, width, rows)
}

/// Opens the finder-path prompt over `root` with `typed` in it.
fn finder_path_prompt(root: &Path, typed: &str) -> App {
    let mut app = editor(root);
    app.working_directory = root.to_path_buf();
    press(&mut app, ' ');
    press(&mut app, '/');
    press(&mut app, 'p');
    type_text(&mut app, typed);
    app
}

#[test]
fn the_finder_path_assistance_is_the_same_list_in_the_same_corner_in_both_renderers() {
    let root = small_directory(&temporary("path-assistance-agreement"));

    let mut standalone = finder_path_prompt(&root, &format!("{}/f", root.display()));
    let drawn = standalone_screen(&mut standalone, 120, 24);
    let published = attached_screen(
        finder_path_prompt(&root, &format!("{}/f", root.display())),
        120,
        24,
    );

    let (drawn_top, drawn_left, drawn_width, drawn_rows) = hint_box(&drawn);
    let (published_top, published_left, published_width, published_rows) = hint_box(&published);
    assert_eq!(
        (drawn_top, drawn_left, drawn_width),
        (published_top, published_left, published_width),
        "the two renderers place the same hints in the same corner at the same size\n\
         standalone:\n{}\nattached:\n{}",
        drawn.join("\n"),
        published.join("\n")
    );
    assert_eq!(drawn_rows, published_rows, "and draw the same rows in it");

    // Bottom left of the editor area, which is the two rows above the status
    // and interaction lines, and no bigger than the two hints need.
    assert_eq!(drawn_left, 0, "at the left edge");
    assert_eq!(
        drawn_top + drawn_rows.len(),
        22,
        "resting on the bottom of the editor area"
    );
    assert_eq!(drawn_rows.len(), 4, "two hints and two borders");
    assert!(
        drawn_width < 120,
        "sized to the entries rather than to the editor: {drawn_width}"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_finder_path_assistance_is_titled_for_the_finder_and_carries_no_query_of_its_own() {
    let root = small_directory(&temporary("path-assistance-title"));
    let typed = format!("{}/f", root.display());

    let mut app = finder_path_prompt(&root, &typed);
    let drawn = standalone_screen(&mut app, 120, 24);
    let (_, _, _, rows) = hint_box(&drawn);

    assert!(
        rows[0].contains("Choose path for finder"),
        "the prompt names what its rows would answer: {}",
        rows[0]
    );
    assert!(
        !rows.iter().any(|row| row.contains("> ")),
        "the interaction line is the query, so the box has no second one: {rows:?}"
    );
    // The typed path stands once, on the interaction line the prompt owns.
    assert_eq!(
        drawn
            .iter()
            .filter(|line| line.contains(&format!("find under path: {typed}")))
            .count(),
        1
    );
    assert!(
        !rows.iter().any(|row| row.contains(&typed)),
        "no row repeats the base already typed: {rows:?}"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_hint_row_shows_the_entry_name_while_tab_still_completes_the_whole_spelling() {
    let root = small_directory(&temporary("path-assistance-rows"));
    let typed = format!("{}/f", root.display());

    let mut app = finder_path_prompt(&root, &typed);
    let drawn = standalone_screen(&mut app, 120, 24);
    let (_, _, _, rows) = hint_box(&drawn);

    let folder = rows
        .iter()
        .find(|row| row.contains("folder/"))
        .expect("the directory is offered");
    assert!(
        folder.contains("▸ folder/  directory"),
        "the row is the name completion would add: {folder}"
    );
    assert!(
        !folder.contains(&typed),
        "and not the base it sits under, which the resolved path would put back: {folder}"
    );

    // Where the typed spelling is relative, the resolved path does say
    // something the name does not, and the detail column keeps it.
    let mut relative = finder_path_prompt(&root, "f");
    let relative = standalone_screen(&mut relative, 120, 24);
    let (_, _, _, relative_rows) = hint_box(&relative);
    let folder = relative_rows
        .iter()
        .find(|row| row.contains("folder/"))
        .expect("the directory is offered");
    assert!(
        folder.contains("directory · ")
            && folder.contains(&format!("{}", root.join("folder").display())),
        "a relative spelling resolves to somewhere worth naming: {folder}"
    );

    // What the row shows is a rendering decision; what Tab inserts is not.
    app.handle_key(KeyStroke::new(KeyCode::Tab, Modifiers::NONE))
        .unwrap();
    assert_eq!(
        app.command,
        format!("{}/folder{}", root.display(), std::path::MAIN_SEPARATOR)
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_palette_path_argument_is_titled_for_the_command_it_completes() {
    let root = small_directory(&temporary("path-assistance-palette"));

    let mut app = editor(&root);
    press(&mut app, ':');
    type_text(&mut app, &format!("open {}/f", root.display()));
    let drawn = standalone_screen(&mut app, 120, 24);
    let (top, left, _, rows) = hint_box(&drawn);

    assert!(
        rows[0].contains("Choose path for :open"),
        "one title serves every path-argument command by naming this one: {}",
        rows[0]
    );
    assert_eq!(left, 0);
    assert_eq!(
        top + rows.len(),
        22,
        "the same corner the finder prompt uses"
    );
    assert!(
        !rows.iter().any(|row| row.contains("> ")),
        "and no query line, because `:open …` is on the interaction line: {rows:?}"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_assistance_stays_bounded_when_a_directory_holds_far_more_than_it_can_show() {
    let root = temporary("path-assistance-bounded");
    let wide = root.join("wide");
    wide_directory(&wide);

    let mut app = finder_path_prompt(&root, &format!("{}/", wide.display()));
    let drawn = standalone_screen(&mut app, 120, 24);
    let (top, _, width, rows) = hint_box(&drawn);

    assert!(
        rows.len() <= 14,
        "a few rows of assistance, not a wall: {}",
        rows.len()
    );
    assert!(width <= 120, "never wider than the editor: {width}");
    assert_eq!(top + rows.len(), 22, "still resting on the same edge");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_assistance_title_keeps_its_keys_when_it_also_counts_the_rows() {
    let root = temporary("path-assistance-title-count");
    fs::create_dir_all(&root).unwrap();
    for index in 0..22 {
        fs::create_dir_all(root.join(format!("d{index:02}"))).unwrap();
    }

    let mut app = editor(&root);
    press(&mut app, ':');
    type_text(&mut app, &format!("cd {}/", root.display()));
    let drawn = standalone_screen(&mut app, 120, 24);
    let (_, _, _, rows) = hint_box(&drawn);

    // The rows are short, so the border is sized from its own title. A list
    // longer than the box adds its position to that title, and the keys are
    // what the border cuts when the count is not paid for.
    assert!(
        rows[0].contains("/23"),
        "a list longer than the box says where in it the rows are: {}",
        rows[0]
    );
    assert!(
        rows[0].contains("Tab complete"),
        "and the keys that operate it survive alongside that count: {}",
        rows[0]
    );

    fs::remove_dir_all(root).unwrap();
}
