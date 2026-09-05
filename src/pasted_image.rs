// SPDX-License-Identifier: MPL-2.0

//! Images pasted from the system clipboard: where the bytes are kept, and
//! what the document calls them.
//!
//! A terminal does not show pictures, so an image reaching a document reaches
//! it as a Markdown reference to a file. This module owns the three decisions
//! that reference embodies — which directory the file belongs in, what it is
//! named, and how the document spells the link — and knows nothing about
//! buffers, panes, or the clipboard it came from.
//!
//! The file is named by the hash of its own bytes rather than by a fresh
//! random identifier. Pasting the same screenshot into two documents, or into
//! one document twice, then costs one file rather than a copy per paste, and a
//! test can name the file it expects instead of hunting the directory for
//! whatever appeared in it.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

/// Directory, relative to the workspace state root, that pasted images live
/// in. Under the default state root this is `.runyte/cache/images`.
const CACHE_DIRECTORY: [&str; 2] = ["cache", "images"];

/// Characters of the content hash a stored file is named with.
///
/// A full SHA-256 is 64 characters, which buries the readable part of the link
/// it appears in. Sixteen hexadecimal characters are 64 bits: enough that a
/// workspace will not see two different images land on one name, and short
/// enough that `[Image 1](.runyte/cache/images/1f0a….png)` still reads as a
/// path.
const NAME_LENGTH: usize = 16;

/// The text a numbered reference is written with, before its number.
const REFERENCE_PREFIX: &str = "Image ";

/// The largest clipboard image Runyte will store.
///
/// The clipboard boundary already bounds what it hands over; this bounds what
/// reaches the workspace, so a wedged helper streaming into `.runyte` cannot
/// fill the disk behind a single keystroke.
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

/// An image format Runyte recognises from the bytes themselves.
///
/// The clipboard says which type it handed over, but the extension a reader
/// and every downstream tool goes by has to match the actual content, so the
/// bytes are the authority here rather than the helper's claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
}

impl ImageFormat {
    /// Every format, so that a name written under one can be recognised again.
    pub const ALL: &'static [Self] = &[Self::Png, Self::Jpeg, Self::Gif, Self::Webp, Self::Bmp];

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Bmp => "bmp",
        }
    }

    /// The format `bytes` begin with, or `None` when they are not an image
    /// this editor is prepared to name.
    pub fn detect(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Some(Self::Png);
        }
        if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            return Some(Self::Jpeg);
        }
        if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            return Some(Self::Gif);
        }
        // RIFF is a container: the four bytes after its length say what is in
        // it, and only `WEBP` is an image.
        if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP" {
            return Some(Self::Webp);
        }
        if bytes.starts_with(b"BM") {
            return Some(Self::Bmp);
        }
        None
    }
}

/// The directory pasted images are stored in for a workspace.
pub fn cache_directory(state_root: &Path) -> PathBuf {
    CACHE_DIRECTORY
        .iter()
        .fold(state_root.to_path_buf(), |path, segment| path.join(segment))
}

/// What a stored image is called, from its own bytes.
pub fn file_name(bytes: &[u8], format: ImageFormat) -> String {
    let digest = crate::hash::sha256_hex(bytes);
    format!("{}.{}", &digest[..NAME_LENGTH], format.extension())
}

/// Writes `bytes` into the workspace image cache and returns the file's path.
///
/// An image already stored under the same name is left exactly as it is: the
/// name is the hash of the content, so a file that is already there already
/// holds these bytes, and rewriting it would only put a torn file where a
/// whole one was. The write goes to a temporary neighbour first and is then
/// renamed, so a paste interrupted partway through leaves no half-written
/// image behind a name that claims to be complete.
pub fn store(state_root: &Path, bytes: &[u8], format: ImageFormat) -> io::Result<PathBuf> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("the clipboard image exceeds {MAX_IMAGE_BYTES} byte(s)"),
        ));
    }
    let directory = cache_directory(state_root);
    fs::create_dir_all(&directory)?;
    discard_abandoned_writes(&directory);
    let path = directory.join(file_name(bytes, format));
    if path.exists() {
        return Ok(path);
    }
    let pending = directory.join(format!(
        ".{}.{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let write = (|| -> io::Result<()> {
        let mut file = fs::File::create(&pending)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = write {
        let _ = fs::remove_file(&pending);
        return Err(error);
    }
    if let Err(error) = fs::rename(&pending, &path) {
        let _ = fs::remove_file(&pending);
        // Another paste of the same image may have completed its own rename in
        // between, which is the outcome this wanted anyway.
        if !path.exists() {
            return Err(error);
        }
    }
    Ok(path)
}

/// How long a half-written image is left alone before it is swept up.
///
/// Long enough that no write still in progress can be mistaken for an
/// abandoned one, including on a filesystem whose clock disagrees with this
/// one by a few minutes.
const ABANDONED_WRITE_AGE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Removes pending files that no write is going to finish.
///
/// Every failure path here removes its own pending file, but a process killed
/// between creating one and renaming it cannot. Nothing else ever prunes this
/// directory, so without this the leftovers accumulate for the life of the
/// workspace. Only this module's own pending spelling is considered, and only
/// once it is far too old to belong to a live write: a sweep that guessed
/// would be deleting somebody's file.
fn discard_abandoned_writes(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_pending_name(name) {
            continue;
        }
        let abandoned = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| {
                modified
                    .elapsed()
                    .is_ok_and(|age| age > ABANDONED_WRITE_AGE)
            });
        if abandoned {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Whether a name is one this module writes an unfinished image under:
/// `.<hash>.<extension>.<pid>`.
fn is_pending_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let mut parts = rest.rsplitn(3, '.');
    let (Some(pid), Some(extension), Some(hash)) = (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    !pid.is_empty()
        && pid.bytes().all(|byte| byte.is_ascii_digit())
        && hash.len() == NAME_LENGTH
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        && ImageFormat::ALL
            .iter()
            .any(|format| format.extension() == extension)
}

/// How a stored image is written into a document.
pub fn reference(number: usize, target: &str) -> String {
    format!("[{REFERENCE_PREFIX}{number}]({target})")
}

/// A path as a Markdown link destination.
///
/// A bare destination ends at the first space or `)`, so a workspace under
/// `~/My Projects` or a directory whose name carries a parenthesis would
/// produce a link that stops partway through its own path. Angle brackets are
/// the spelling for that case, and they are used only when they are needed:
/// the overwhelmingly common destination is a plain relative path, and
/// wrapping every one of them would make the source harder to read for the
/// sake of the rare one.
pub fn destination(path: &str) -> String {
    if path.contains([' ', '\t', '(', ')', '<', '>']) {
        // Either bracket inside the path breaks the wrapper it sits in: `>`
        // would close it early, and a second `<` is what tells a reader the
        // first one never opened a destination at all. There is no escape for
        // either in a destination, so both are percent-encoded, which every
        // reader of these links already resolves. Neither replacement can
        // introduce the other's character, so the order does not matter.
        return format!("<{}>", path.replace('<', "%3C").replace('>', "%3E"));
    }
    path.to_owned()
}

/// The number the next image pasted into a document should carry.
///
/// Numbering continues from the highest `[Image N]` the document already
/// holds rather than from how many it holds, so deleting the second of three
/// images does not make the next paste collide with the third. The document is
/// the only state this reads, which is what keeps the numbers right after the
/// file has been closed, reopened, or edited by something else entirely.
///
/// It takes lines rather than the whole text because the caller is normally a
/// rope: a reference never spans a line break, so nothing is lost by never
/// assembling the document into one string to look at it.
pub fn next_number(lines: impl IntoIterator<Item = impl AsRef<str>>) -> usize {
    lines
        .into_iter()
        .filter_map(|line| highest_in_line(line.as_ref()))
        .max()
        .map_or(1, |highest| highest.saturating_add(1))
}

fn highest_in_line(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let opening = format!("[{REFERENCE_PREFIX}");
    let mut highest = None;
    let mut index = 0;
    while let Some(offset) = find(&bytes[index..], opening.as_bytes()) {
        let start = index + offset + opening.len();
        index = start;
        let digits = bytes[start..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        // Only a complete `[Image N](` counts. Prose that happens to mention
        // "[Image 4" without making a link of it is not a reference this has
        // to number past.
        if digits == 0
            || bytes.get(start + digits) != Some(&b']')
            || bytes.get(start + digits + 1) != Some(&b'(')
        {
            continue;
        }
        // A number too long to be one is not a reference to count past; the
        // document keeps it and the next paste ignores it.
        if let Ok(number) = line[start..start + digits].parse::<usize>() {
            highest = Some(highest.map_or(number, |highest: usize| highest.max(number)));
        }
    }
    highest
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "runyte-pasted-image-{name}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn formats_are_recognised_from_their_own_leading_bytes() {
        assert_eq!(
            ImageFormat::detect(b"\x89PNG\r\n\x1a\n\x00\x00"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            ImageFormat::detect(&[0xff, 0xd8, 0xff, 0xe0]),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(ImageFormat::detect(b"GIF89a...."), Some(ImageFormat::Gif));
        assert_eq!(
            ImageFormat::detect(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            Some(ImageFormat::Webp)
        );
        assert_eq!(ImageFormat::detect(b"BM\x00\x00"), Some(ImageFormat::Bmp));

        // A RIFF container that is not a picture, and plain text, are not
        // images however much of them is read.
        assert_eq!(ImageFormat::detect(b"RIFF\x00\x00\x00\x00WAVEfmt "), None);
        assert_eq!(ImageFormat::detect(b"not an image at all"), None);
        assert_eq!(ImageFormat::detect(b""), None);
        // A truncated signature is not a match for the format it begins like.
        assert_eq!(ImageFormat::detect(b"RIFF\x00\x00"), None);
    }

    #[test]
    fn a_stored_image_is_named_by_its_content_and_written_once() {
        let root = temporary_root("store");
        let bytes = b"\x89PNG\r\n\x1a\nfirst".to_vec();

        let path = store(&root, &bytes, ImageFormat::Png).unwrap();
        assert_eq!(
            path,
            root.join("cache/images")
                .join(file_name(&bytes, ImageFormat::Png))
        );
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".png")
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);

        // The same bytes name the same file, and storing them again neither
        // fails nor writes a second copy.
        let again = store(&root, &bytes, ImageFormat::Png).unwrap();
        assert_eq!(again, path);
        assert_eq!(
            fs::read_dir(root.join("cache/images")).unwrap().count(),
            1,
            "an identical paste created a second file"
        );

        // Different bytes get a different name.
        let other = store(&root, b"\x89PNG\r\n\x1a\nsecond", ImageFormat::Png).unwrap();
        assert_ne!(other, path);

        fs::remove_dir_all(root).unwrap();
    }

    /// A process killed between writing a pending file and renaming it cannot
    /// clean up after itself, and nothing else prunes this directory.
    #[test]
    fn an_abandoned_write_is_swept_up_and_nothing_else_is() {
        let root = temporary_root("abandoned");
        let directory = cache_directory(&root);
        fs::create_dir_all(&directory).unwrap();

        let abandoned = directory.join(".0123456789abcdef.png.4242");
        let recent = directory.join(".fedcba9876543210.png.4243");
        let stored = directory.join("0123456789abcdef.png");
        let unrelated = directory.join(".notes.txt.1");
        for path in [&abandoned, &recent, &stored, &unrelated] {
            fs::write(path, b"x").unwrap();
        }
        let old = std::time::SystemTime::now() - ABANDONED_WRITE_AGE * 2;
        // Opened for writing rather than read: setting a timestamp needs
        // write access to the file's attributes on Windows, and a read-only
        // handle is refused there while succeeding here.
        let age = |path: &Path| {
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(old)
                .unwrap();
        };
        age(&abandoned);
        // An unrelated file is not swept whatever its age.
        age(&unrelated);

        store(&root, b"\x89PNG\r\n\x1a\nsweep", ImageFormat::Png).unwrap();

        assert!(!abandoned.exists(), "an abandoned write survived");
        assert!(
            recent.exists(),
            "a write that may still be running was swept"
        );
        assert!(stored.exists(), "a stored image was swept");
        assert!(
            unrelated.exists(),
            "a file this module never wrote was swept"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_this_modules_own_pending_spelling_is_recognised() {
        assert!(is_pending_name(".0123456789abcdef.png.1"));
        assert!(is_pending_name(".0123456789ABCDEF.webp.99999"));
        for other in [
            "0123456789abcdef.png.1",  // not hidden
            ".0123456789abcdef.png",   // no pid
            ".0123456789abcdef.png.a", // pid is not a number
            ".0123456789abcdef.txt.1", // not an image extension
            ".0123456789abcde.png.1",  // hash is the wrong length
            ".0123456789abcdeg.png.1", // hash is not hexadecimal
            ".png.1",
            ".",
            "",
        ] {
            assert!(
                !is_pending_name(other),
                "{other} was taken for a pending write"
            );
        }
    }

    #[test]
    fn an_oversized_image_is_refused_rather_than_written() {
        let root = temporary_root("oversized");
        let bytes = vec![0_u8; MAX_IMAGE_BYTES + 1];
        let error = store(&root, &bytes, ImageFormat::Png).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!cache_directory(&root).exists());
        fs::remove_dir_all(root).unwrap();
    }

    fn numbering(document: &str) -> usize {
        next_number(document.lines())
    }

    #[test]
    fn numbering_continues_past_the_highest_reference_the_document_holds() {
        assert_eq!(numbering(""), 1);
        assert_eq!(numbering("no images here"), 1);
        assert_eq!(numbering("[Image 1](a.png)"), 2);
        // The highest wins rather than the last or the count, so removing a
        // middle reference cannot make the next paste collide with a later one.
        assert_eq!(
            numbering("[Image 1](a.png) and [Image 7](b.png) and [Image 3](c.png)"),
            8
        );
        // References are found wherever they sit, including one per line.
        assert_eq!(numbering("[Image 2](a.png)\ntext\n[Image 5](b.png)\n"), 6);
        // Prose that mentions the words is not a reference.
        assert_eq!(numbering("see [Image 9] in the appendix"), 1);
        assert_eq!(numbering("[Image ](a.png)"), 1);
        assert_eq!(numbering("[Images 4](a.png)"), 1);
        // A number no machine can hold is left alone rather than counted past.
        assert_eq!(numbering("[Image 999999999999999999999999](a.png)"), 1);
    }

    #[test]
    fn a_destination_is_bracketed_only_when_it_has_to_be() {
        assert_eq!(
            destination(".runyte/cache/images/ab.png"),
            ".runyte/cache/images/ab.png"
        );
        // A space or a parenthesis would otherwise end the destination early.
        assert_eq!(destination("My Notes/a.png"), "<My Notes/a.png>");
        assert_eq!(
            destination("plans/draft (old)/a.png"),
            "<plans/draft (old)/a.png>"
        );
        // Neither bracket may be left inside the wrapper it sits in: `>`
        // would close it early, and `<` is what says it never opened.
        assert_eq!(destination("odd>name/a.png"), "<odd%3Ename/a.png>");
        assert_eq!(destination("odd<name/a.png"), "<odd%3Cname/a.png>");
        assert_eq!(destination("odd<>name/a.png"), "<odd%3C%3Ename/a.png>");
    }

    #[test]
    fn a_reference_is_the_link_the_renderer_reads() {
        assert_eq!(
            reference(2, ".runyte/cache/images/abc.png"),
            "[Image 2](.runyte/cache/images/abc.png)"
        );
        assert_eq!(numbering(&reference(2, "a.png")), 3);
    }
}
