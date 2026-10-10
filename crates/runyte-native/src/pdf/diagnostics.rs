// SPDX-License-Identifier: MPL-2.0

//! Hayro reports some font substitutions and malformed content through `log`
//! rather than WarningSink. Capture a flag, never unbounded document-derived
//! log strings. This logger is installed only in the disposable helper/tests.
use std::{cell::Cell, sync::OnceLock};
thread_local! {
    static FAILED: Cell<bool> = const { Cell::new(false) };
}
struct Diagnostics;
impl log::Log for Diagnostics {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn && metadata.target().starts_with("hayro")
    }
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            FAILED.set(true);
        }
    }
    fn flush(&self) {}
}
pub(super) struct Scope;
impl Scope {
    pub fn new() -> anyhow::Result<Self> {
        static INSTALLED: OnceLock<bool> = OnceLock::new();
        anyhow::ensure!(
            *INSTALLED.get_or_init(|| {
                if log::set_logger(&Diagnostics).is_err() {
                    return false;
                }
                log::set_max_level(log::LevelFilter::Warn);
                true
            }),
            "PDF helper diagnostics unavailable"
        );
        FAILED.set(false);
        Ok(Self)
    }
    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !FAILED.get(),
            "Hayro reported unsupported or damaged PDF font/content; use Poppler"
        );
        Ok(())
    }
}
