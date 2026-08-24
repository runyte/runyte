// SPDX-License-Identifier: MPL-2.0

//! Recently read directory listings, kept so that completing a path does not
//! re-read the same directory for every keystroke and every redraw.
//!
//! Path completion has to look at a directory whole. A name the person is
//! typing can sit anywhere in it, and a directory read returns names in
//! whatever order the filesystem holds them, so anything less than the whole
//! listing hides matches for no reason a person could see. On a directory of a
//! hundred thousand entries that read is tens of milliseconds — affordable
//! once, but not once per keystroke, and certainly not once per frame, which
//! is what the command palette would otherwise ask for.
//!
//! So the listing is kept, and reuse costs one `stat` instead. Whether it may
//! be reused is answered in two ways, because one of them is not always
//! available:
//!
//! - The directory's own modification time, which the filesystem updates when
//!   an entry is created, removed, or renamed — exactly the changes that make
//!   a listing wrong. When it is unchanged, and was already old enough when
//!   the listing was read that a change could not have hidden inside the same
//!   clock tick, the listing is good for as long as that stays true.
//! - Otherwise — a directory written to moments before it was listed, on a
//!   filesystem that records modification times to a whole second, or one that
//!   will not report a modification time at all — the listing is reused only
//!   for a short window after the read. That bounds how stale an answer can be
//!   to about the length of that window, while still collapsing the burst of
//!   requests one keystroke and its redraw make into a single read. Refusing
//!   to reuse it at all instead would mean a full synchronous read per
//!   keystroke in exactly the directory that can least afford one: the one
//!   that has just been written to.
//!
//! A changed modification time is decisive either way: a listing known to be
//! out of date is never reused, however recently it was read.
//!
//! One thing a directory's modification time cannot describe is what a symlink
//! inside it points at, which is where the kind of a linked name comes from.
//! Those kinds are therefore asked again whenever a listing is reused. That
//! costs one `stat` per link rather than per entry, and never more than the
//! read it stands in for: reading the directory again would ask the same
//! question about the same links, and pay for the directory read besides.
//!
//! Knows nothing about buffers, panes, or what makes a name worth offering.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

/// One name in a directory, with the one property completion needs of it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub name: String,
    /// Whether the name leads somewhere, following a symlink the way
    /// [`Path::is_dir`] does.
    pub is_directory: bool,
}

/// How old a directory's modification time has to be when its listing is read
/// for equality to prove, later, that nothing has changed in it.
///
/// Covers a filesystem that records modification times to a whole second: a
/// change made in the same second as the read leaves the recorded time equal
/// to the one already held, so equality alone would not notice it.
const SETTLED: Duration = Duration::from_secs(2);

/// How long a listing whose modification time cannot vouch for it is reused
/// anyway.
///
/// At least [`SETTLED`], so that a directory written to just before it was
/// listed costs one extra read rather than one per keystroke: by the time the
/// window is out, the same directory left alone is old enough for its
/// modification time to take over.
const VOLATILE: Duration = SETTLED;

/// How many distinct directories are kept.
///
/// Completing one path consults at most two, and moving between directories
/// is what typing a path does, so a handful covers the descent without
/// holding listings nobody will ask for again.
const DIRECTORIES: usize = 4;

/// How many entries the kept listings hold between them before the oldest are
/// dropped.
///
/// The most recent listing is always kept, however large it is. A directory
/// big enough to exceed this on its own is the one where re-reading hurts
/// most, so it is held even though it costs more memory than the rest of the
/// cache is allowed together; what the bound then governs is how many *other*
/// listings are kept beside it.
const ENTRIES: usize = 250_000;

struct Cached {
    directory: PathBuf,
    /// The directory's modification time when the listing was read. Absent
    /// when the platform would not say, which leaves the window below as the
    /// only thing vouching for this listing.
    modified: Option<SystemTime>,
    /// Whether `modified` was already older than [`SETTLED`] when the listing
    /// was read, and so can prove on its own that nothing has changed since.
    settled: bool,
    /// When the listing was read, for the window during which it is reused
    /// even though `modified` cannot vouch for it.
    read_at: SystemTime,
    /// Positions in `entries` whose kind came from following a symlink, and
    /// so has to be asked again on reuse.
    symlinks: Vec<usize>,
    entries: Arc<[Entry]>,
}

/// A bounded, most-recently-used cache of directory listings.
#[derive(Default)]
pub struct DirectoryListings {
    /// Most recently used last.
    cached: Vec<Cached>,
}

impl DirectoryListings {
    /// The entries of `directory`, read from the filesystem only when no kept
    /// listing still describes it.
    ///
    /// `None` means the directory could not be read at all, which callers
    /// treat the same as a directory with nothing in it to offer.
    pub fn read(&mut self, directory: &Path) -> Option<Arc<[Entry]>> {
        self.read_at(directory, SystemTime::now())
    }

    /// The same read against a stated present, so that the rules above can be
    /// exercised without waiting for a real clock or rewriting a directory's
    /// recorded times, which not every platform allows.
    fn read_at(&mut self, directory: &Path, now: SystemTime) -> Option<Arc<[Entry]>> {
        let modified = fs::metadata(directory)
            .and_then(|metadata| metadata.modified())
            .ok();
        if let Some(index) = self
            .cached
            .iter()
            .position(|cached| cached.directory == directory)
        {
            let mut cached = self.cached.remove(index);
            // A modification time that has moved is proof the listing is out
            // of date. One that has not moved proves nothing unless it was
            // already settled when the listing was read, which is what the
            // window covers for.
            // Losing metadata is a change too: a settled directory may have
            // been renamed or removed since it was listed. Treating
            // `Some(_) -> None` as unchanged would let that old listing live
            // forever on the strength of the timestamp we can no longer
            // verify. Gaining metadata after an unverified read likewise
            // earns a fresh listing rather than spending the rest of the
            // volatile window on weaker evidence.
            let changed = cached.modified != modified;
            let within_window = now
                .duration_since(cached.read_at)
                .is_ok_and(|age| age < VOLATILE);
            if !changed && (cached.settled || within_window) {
                refresh_symlinks(&mut cached);
                let entries = Arc::clone(&cached.entries);
                self.cached.push(cached);
                return Some(entries);
            }
        }

        let mut entries = Vec::new();
        let mut symlinks = Vec::new();
        for entry in fs::read_dir(directory).ok()?.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let kind = entry_kind(&entry);
            if kind.followed_a_symlink {
                symlinks.push(entries.len());
            }
            entries.push(Entry {
                name,
                is_directory: kind.is_directory,
            });
        }
        let entries: Arc<[Entry]> = Arc::from(entries);

        let settled = modified
            .is_some_and(|modified| now.duration_since(modified).is_ok_and(|age| age >= SETTLED));
        self.cached.push(Cached {
            directory: directory.to_path_buf(),
            modified,
            settled,
            read_at: now,
            symlinks,
            entries: Arc::clone(&entries),
        });
        evict(&mut self.cached);
        Some(entries)
    }
}

/// Drops the least recently used listings until the cache is back inside its
/// bounds, always leaving the most recent one in place.
fn evict(cached: &mut Vec<Cached>) {
    while cached.len() > DIRECTORIES
        || (cached.len() > 1
            && cached
                .iter()
                .map(|cached| cached.entries.len())
                .sum::<usize>()
                > ENTRIES)
    {
        cached.remove(0);
    }
}

/// Asks again what each symlinked name in a kept listing points at, replacing
/// the listing only when one of them now says something different.
///
/// The containing directory's modification time does not move when a link's
/// target is created or removed elsewhere, so this is the one part of a
/// listing that being unchanged cannot vouch for.
fn refresh_symlinks(cached: &mut Cached) {
    let changed = cached
        .symlinks
        .iter()
        .filter_map(|index| {
            let entry = cached.entries.get(*index)?;
            let is_directory = cached.directory.join(&entry.name).is_dir();
            (is_directory != entry.is_directory).then_some((*index, is_directory))
        })
        .collect::<Vec<_>>();
    if changed.is_empty() {
        return;
    }
    let mut entries = cached.entries.to_vec();
    for (index, is_directory) in changed {
        entries[index].is_directory = is_directory;
    }
    cached.entries = Arc::from(entries);
}

/// What a directory entry names, and whether saying so meant following a
/// symlink.
struct EntryKind {
    is_directory: bool,
    followed_a_symlink: bool,
}

/// Whether a directory entry names a directory, following a symlink the way
/// [`Path::is_dir`] does.
///
/// `DirEntry::file_type` is answered from what the directory read already
/// returned wherever the platform carries the kind inline, so listing a very
/// large directory does not become one `stat` per entry. Only a symlink,
/// whose target the entry cannot describe, still costs one.
fn entry_kind(entry: &fs::DirEntry) -> EntryKind {
    match entry.file_type() {
        Ok(file_type) if !file_type.is_symlink() => EntryKind {
            is_directory: file_type.is_dir(),
            followed_a_symlink: false,
        },
        _ => EntryKind {
            is_directory: entry.path().is_dir(),
            followed_a_symlink: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    fn temporary(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "runyte-directory-listing-{}-{nanos}-{name}",
            std::process::id()
        ))
    }

    /// A present far enough past a just-written directory that its
    /// modification time can vouch for a listing on its own, without waiting
    /// out the settling window a coarse filesystem clock needs.
    fn later() -> SystemTime {
        SystemTime::now() + Duration::from_secs(60)
    }

    #[test]
    fn an_unchanged_directory_is_read_once() {
        let root = temporary("unchanged");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "").unwrap();

        let mut listings = DirectoryListings::default();
        let first = listings.read_at(&root, later()).unwrap();
        let second = listings.read_at(&root, later()).unwrap();
        assert_eq!(first.len(), 1);
        assert!(
            Arc::ptr_eq(&first, &second),
            "an unchanged directory should be answered from the kept listing"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_entry_invalidates_the_kept_listing() {
        let root = temporary("changed");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "").unwrap();

        let mut listings = DirectoryListings::default();
        let first = listings.read_at(&root, later()).unwrap();
        assert_eq!(first.len(), 1);

        fs::write(root.join("b.txt"), "").unwrap();
        let second = listings.read_at(&root, later()).unwrap();
        assert_eq!(second.len(), 2);
        assert!(!Arc::ptr_eq(&first, &second));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_directory_that_disappears_does_not_return_its_kept_listing() {
        let root = temporary("removed");
        let listed = root.join("listed");
        let moved = root.join("moved");
        fs::create_dir_all(&listed).unwrap();
        fs::write(listed.join("a.txt"), "").unwrap();

        let mut listings = DirectoryListings::default();
        let first = listings.read_at(&listed, later()).unwrap();
        assert_eq!(first.len(), 1);

        // Renaming the directory makes metadata at the cached path
        // unavailable. The old timestamp must not keep vouching for a path
        // that no longer names a directory.
        fs::rename(&listed, &moved).unwrap();
        assert!(listings.read_at(&listed, later()).is_none());

        fs::remove_dir_all(root).unwrap();
    }

    /// A directory written to moments before it was listed is the case a
    /// modification time cannot describe. Reading it again for every request
    /// would mean a full read per keystroke in exactly the directory that can
    /// least afford one, so the listing is reused for a bounded window and
    /// then read once more.
    #[test]
    fn a_directory_touched_moments_ago_is_reused_for_a_bounded_window() {
        let root = temporary("unsettled");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "").unwrap();

        let now = SystemTime::now();
        let mut listings = DirectoryListings::default();
        let first = listings.read_at(&root, now).unwrap();
        let within = listings.read_at(&root, now + VOLATILE / 2).unwrap();
        assert!(
            Arc::ptr_eq(&first, &within),
            "a request inside the window should not read the directory again"
        );

        let after = listings.read_at(&root, now + VOLATILE).unwrap();
        assert!(
            !Arc::ptr_eq(&first, &after),
            "the window has to end in a read rather than in a stale listing"
        );
        // That read found the directory settled, so from here its
        // modification time vouches for the listing and no further read is
        // needed.
        let settled = listings.read_at(&root, now + VOLATILE * 2).unwrap();
        assert!(Arc::ptr_eq(&after, &settled));

        fs::remove_dir_all(root).unwrap();
    }

    /// A modification time that has moved is decisive even inside the window.
    #[test]
    fn a_change_inside_the_window_is_still_noticed() {
        let root = temporary("changed-inside-window");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "").unwrap();

        let now = SystemTime::now();
        let mut listings = DirectoryListings::default();
        let first = listings.read_at(&root, now).unwrap();
        // Waited out by the filesystem's own clock rather than by ours: the
        // point is a modification time that differs from the one held.
        std::thread::sleep(Duration::from_millis(20));
        fs::write(root.join("b.txt"), "").unwrap();
        let second = listings.read_at(&root, now + VOLATILE / 2).unwrap();

        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(second.len(), 2);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn kept_listings_stay_bounded_and_hold_the_most_recent_directories() {
        let root = temporary("bounded");
        let mut directories = Vec::new();
        for index in 0..DIRECTORIES + 2 {
            let directory = root.join(format!("d{index}"));
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("a.txt"), "").unwrap();
            directories.push(directory);
        }

        let mut listings = DirectoryListings::default();
        let oldest = listings.read_at(&directories[0], later()).unwrap();
        for directory in &directories[1..] {
            listings.read_at(directory, later()).unwrap();
        }
        assert_eq!(listings.cached.len(), DIRECTORIES);
        assert!(
            !Arc::ptr_eq(
                &oldest,
                &listings.read_at(&directories[0], later()).unwrap()
            ),
            "the least recently used directory should have been dropped"
        );
        // The most recent read is still held.
        let last = directories.last().unwrap();
        let held = listings.read_at(last, later()).unwrap();
        assert!(Arc::ptr_eq(
            &held,
            &listings.read_at(last, later()).unwrap()
        ));

        fs::remove_dir_all(root).unwrap();
    }

    /// A directory too large for the cache's own entry bound is still kept,
    /// because it is the one where reading again hurts most. What the bound
    /// governs is how many other listings are kept beside it.
    #[test]
    fn a_listing_larger_than_the_entry_bound_is_still_kept() {
        let fabricate = |name: &str, entries: usize| Cached {
            directory: PathBuf::from(name),
            modified: None,
            settled: false,
            read_at: SystemTime::now(),
            symlinks: Vec::new(),
            entries: Arc::from(vec![
                Entry {
                    name: String::new(),
                    is_directory: false,
                };
                entries
            ]),
        };

        let mut cached = vec![fabricate("small", 4), fabricate("huge", ENTRIES + 1)];
        evict(&mut cached);
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].directory, PathBuf::from("huge"));

        // And it is not kept at the cost of the bound itself: another listing
        // arriving after it drops the oversized one rather than the new one.
        cached.push(fabricate("next", 4));
        evict(&mut cached);
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].directory, PathBuf::from("next"));
    }

    #[test]
    fn an_unreadable_directory_yields_nothing() {
        let root = temporary("missing");
        let mut listings = DirectoryListings::default();
        assert!(listings.read(&root.join("nowhere")).is_none());
    }

    #[test]
    fn entries_report_directories_files_and_symlinked_directories() {
        let root = temporary("kinds");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("file.txt"), "").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("sub"), root.join("link")).unwrap();

        let mut listings = DirectoryListings::default();
        let entries = listings.read(&root).unwrap();
        let kind = |name: &str| {
            entries
                .iter()
                .find(|entry| entry.name == name)
                .unwrap_or_else(|| panic!("{name} should be listed"))
                .is_directory
        };
        assert!(kind("sub"));
        assert!(!kind("file.txt"));
        #[cfg(unix)]
        assert!(kind("link"), "a symlink to a directory is a directory");

        fs::remove_dir_all(root).unwrap();
    }

    /// What a symlink points at can change without the directory holding it
    /// changing at all, so a kept listing has to ask again rather than answer
    /// from the kind it recorded.
    #[cfg(unix)]
    #[test]
    fn a_symlink_that_gains_or_loses_its_target_is_reclassified() {
        let root = temporary("symlink-target");
        let listed = root.join("listed");
        let target = root.join("target");
        fs::create_dir_all(&listed).unwrap();
        std::os::unix::fs::symlink(&target, listed.join("link")).unwrap();

        let mut listings = DirectoryListings::default();
        let kind = |listings: &mut DirectoryListings| {
            listings
                .read_at(&listed, later())
                .unwrap()
                .iter()
                .find(|entry| entry.name == "link")
                .expect("the link should be listed")
                .is_directory
        };
        assert!(!kind(&mut listings), "a dangling link leads nowhere");

        // The target is created beside the listed directory, so the listed
        // directory's own modification time does not move.
        fs::create_dir_all(&target).unwrap();
        assert!(
            kind(&mut listings),
            "a link whose target now exists leads to a directory"
        );

        fs::remove_dir_all(&target).unwrap();
        assert!(
            !kind(&mut listings),
            "a link whose target is gone leads nowhere again"
        );

        fs::remove_dir_all(root).unwrap();
    }
}
