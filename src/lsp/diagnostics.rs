// SPDX-License-Identifier: MPL-2.0

//! Diagnostics published by language servers.
//!
//! Diagnostics are kept in the server's own coordinates and converted to buffer
//! offsets only where they are rendered or navigated to. Versioned
//! publications are accepted only for the matching open-document version;
//! unversioned publications are converted late, so an asynchronous range is
//! clamped into the current document rather than pointing past its end.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Severity, ordered so that `max` picks the one worth showing in a gutter
/// cell shared by several diagnostics.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    Hint,
    Information,
    Warning,
    Error,
}

impl Severity {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "info",
            Self::Hint => "hint",
        }
    }

    /// The single-cell gutter sign. ASCII, because a diagnostic sign column
    /// must not shift the text by one cell on terminals with partial Unicode
    /// coverage.
    pub const fn sign(self) -> char {
        match self {
            Self::Error => 'E',
            Self::Warning => 'W',
            Self::Information => 'I',
            Self::Hint => 'H',
        }
    }

    fn from_lsp(value: Option<lsp_types::DiagnosticSeverity>) -> Self {
        match value {
            Some(lsp_types::DiagnosticSeverity::WARNING) => Self::Warning,
            Some(lsp_types::DiagnosticSeverity::INFORMATION) => Self::Information,
            Some(lsp_types::DiagnosticSeverity::HINT) => Self::Hint,
            // An absent severity means "error" per the specification.
            _ => Self::Error,
        }
    }
}

/// One published diagnostic.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
    pub range: lsp_types::Range,
    /// The original, kept verbatim because `textDocument/codeAction` must echo
    /// diagnostics back to the server exactly as they were received.
    pub raw: lsp_types::Diagnostic,
}

impl Diagnostic {
    pub fn new(raw: lsp_types::Diagnostic) -> Self {
        Self {
            severity: Severity::from_lsp(raw.severity),
            // Multi-line messages are common; the editor shows diagnostics in
            // one-line contexts, so flatten once here rather than at each site.
            message: raw
                .message
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
            source: raw.source.clone(),
            range: raw.range,
            raw,
        }
    }

    pub fn row(&self) -> usize {
        self.range.start.line as usize
    }

    /// A one-line description, as shown inline and in the diagnostics picker.
    pub fn label(&self) -> String {
        match &self.source {
            Some(source) => format!("{} [{source}] {}", self.severity.label(), self.message),
            None => format!("{} {}", self.severity.label(), self.message),
        }
    }
}

/// Diagnostics for every file a server has reported on.
///
/// Servers publish per file, replacing the previous set, and publish an empty
/// list to clear one. The store mirrors exactly that.
#[derive(Clone, Debug, Default)]
pub struct DiagnosticStore {
    by_path: BTreeMap<PathBuf, Vec<Diagnostic>>,
    by_language: BTreeMap<PathBuf, String>,
}

impl DiagnosticStore {
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    pub fn set(&mut self, language: &str, path: PathBuf, diagnostics: Vec<Diagnostic>) {
        if diagnostics.is_empty() {
            self.by_path.remove(&path);
            self.by_language.remove(&path);
            return;
        }
        self.by_language.insert(path.clone(), language.to_owned());
        self.by_path.insert(path, diagnostics);
    }

    /// Drops everything a language's server published. Used when that server
    /// stops, because diagnostics with no server behind them are stale claims
    /// about the code that nothing will ever correct.
    pub fn clear_language(&mut self, language: &str) {
        let paths: Vec<PathBuf> = self
            .by_language
            .iter()
            .filter(|(_, owner)| owner.as_str() == language)
            .map(|(path, _)| path.clone())
            .collect();
        for path in paths {
            self.by_path.remove(&path);
            self.by_language.remove(&path);
        }
    }

    /// Drops the last diagnostics published for one document path.
    pub fn clear_path(&mut self, path: &Path) {
        self.by_path.remove(path);
        self.by_language.remove(path);
    }

    pub fn for_path(&self, path: &Path) -> &[Diagnostic] {
        self.by_path.get(path).map_or(&[], Vec::as_slice)
    }

    /// Diagnostics on one row, most severe first.
    pub fn for_row(&self, path: &Path, row: usize) -> Vec<&Diagnostic> {
        let mut matching: Vec<&Diagnostic> = self
            .for_path(path)
            .iter()
            .filter(|diagnostic| diagnostic.row() == row)
            .collect();
        matching.sort_by_key(|diagnostic| std::cmp::Reverse(diagnostic.severity));
        matching
    }

    /// The sign a row's gutter cell should carry, if any.
    pub fn severity_for_row(&self, path: &Path, row: usize) -> Option<Severity> {
        self.for_path(path)
            .iter()
            .filter(|diagnostic| diagnostic.row() == row)
            .map(|diagnostic| diagnostic.severity)
            .max()
    }

    /// Every diagnostic in the store, ordered by path and then position, for
    /// the workspace diagnostics picker.
    pub fn all(&self) -> Vec<(&Path, &Diagnostic)> {
        let mut entries: Vec<(&Path, &Diagnostic)> = self
            .by_path
            .iter()
            .flat_map(|(path, diagnostics)| {
                diagnostics
                    .iter()
                    .map(move |diagnostic| (path.as_path(), diagnostic))
            })
            .collect();
        entries.sort_by_key(|(path, diagnostic)| {
            (
                *path,
                diagnostic.range.start.line,
                diagnostic.range.start.character,
            )
        });
        entries
    }

    /// Error and warning totals, for the status line.
    pub fn counts(&self) -> (usize, usize) {
        let mut errors = 0;
        let mut warnings = 0;
        for diagnostic in self.by_path.values().flatten() {
            match diagnostic.severity {
                Severity::Error => errors += 1,
                Severity::Warning => warnings += 1,
                _ => {}
            }
        }
        (errors, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{DiagnosticSeverity, Position, Range};

    fn diagnostic(row: u32, severity: DiagnosticSeverity, message: &str) -> Diagnostic {
        Diagnostic::new(lsp_types::Diagnostic {
            range: Range::new(Position::new(row, 0), Position::new(row, 4)),
            severity: Some(severity),
            message: message.to_owned(),
            ..Default::default()
        })
    }

    #[test]
    fn publishing_an_empty_list_clears_a_file() {
        let mut store = DiagnosticStore::default();
        let path = PathBuf::from("/tmp/a.rs");
        store.set(
            "rust",
            path.clone(),
            vec![diagnostic(0, DiagnosticSeverity::ERROR, "boom")],
        );
        assert_eq!(store.for_path(&path).len(), 1);
        store.set("rust", path.clone(), Vec::new());
        assert!(store.for_path(&path).is_empty());
        assert!(store.is_empty());
    }

    #[test]
    fn a_row_reports_its_most_severe_diagnostic() {
        let mut store = DiagnosticStore::default();
        let path = PathBuf::from("/tmp/a.rs");
        store.set(
            "rust",
            path.clone(),
            vec![
                diagnostic(3, DiagnosticSeverity::HINT, "hint"),
                diagnostic(3, DiagnosticSeverity::ERROR, "error"),
                diagnostic(4, DiagnosticSeverity::WARNING, "warning"),
            ],
        );
        assert_eq!(store.severity_for_row(&path, 3), Some(Severity::Error));
        assert_eq!(store.severity_for_row(&path, 4), Some(Severity::Warning));
        assert_eq!(store.severity_for_row(&path, 5), None);
        assert_eq!(store.for_row(&path, 3)[0].severity, Severity::Error);
        assert_eq!(store.counts(), (1, 1));
    }

    #[test]
    fn stopping_a_server_clears_only_its_own_files() {
        let mut store = DiagnosticStore::default();
        store.set(
            "rust",
            PathBuf::from("/tmp/a.rs"),
            vec![diagnostic(0, DiagnosticSeverity::ERROR, "rust")],
        );
        store.set(
            "json",
            PathBuf::from("/tmp/b.json"),
            vec![diagnostic(0, DiagnosticSeverity::ERROR, "json")],
        );
        store.clear_language("rust");
        assert!(store.for_path(Path::new("/tmp/a.rs")).is_empty());
        assert_eq!(store.for_path(Path::new("/tmp/b.json")).len(), 1);
    }

    #[test]
    fn a_missing_severity_is_an_error() {
        let raw = lsp_types::Diagnostic {
            range: Range::default(),
            message: "unspecified".to_owned(),
            ..Default::default()
        };
        assert_eq!(Diagnostic::new(raw).severity, Severity::Error);
    }

    #[test]
    fn multi_line_messages_are_flattened_for_one_line_surfaces() {
        let raw = lsp_types::Diagnostic {
            range: Range::default(),
            message: "expected `i32`\n   found `&str`".to_owned(),
            ..Default::default()
        };
        assert_eq!(Diagnostic::new(raw).message, "expected `i32` found `&str`");
    }
}
