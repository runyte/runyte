// SPDX-License-Identifier: MPL-2.0

//! Optional startup-phase instrumentation.
//!
//! The default build keeps [`StartupTrace`] zero-sized and compiles every
//! mark into an inline no-op. Measurement builds enable the
//! `startup-timing` feature and may request a report by setting
//! `RUNYTE_STARTUP_TIMING_FILE` to an explicit output path.

use std::{io, path::PathBuf};

#[cfg(feature = "startup-timing")]
use std::{
    env, fs,
    time::{Duration, Instant},
};

pub const STARTUP_TIMING_FILE_ENV: &str = "RUNYTE_STARTUP_TIMING_FILE";

/// Stable milestones on the path to a usable editor frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupPhase {
    MainEntry,
    CliParsed,
    ConfigLoaded,
    ProjectResolutionStarted,
    ProjectResolvedAutomatically,
    ProjectResolvedAfterPrompt,
    ThemeResolved,
    LanguageRegistryReady,
    InitialBufferOpened,
    InitialSyntaxReady,
    AppReady,
    TerminalEntered,
    LspManagerSpawned,
    FirstFramePresented,
}

impl StartupPhase {
    pub const fn name(self) -> &'static str {
        match self {
            Self::MainEntry => "main_entry",
            Self::CliParsed => "cli_parsed",
            Self::ConfigLoaded => "config_loaded",
            Self::ProjectResolutionStarted => "project_resolution_started",
            Self::ProjectResolvedAutomatically => "project_resolved_automatically",
            Self::ProjectResolvedAfterPrompt => "project_resolved_after_prompt",
            Self::ThemeResolved => "theme_resolved",
            Self::LanguageRegistryReady => "language_registry_ready",
            Self::InitialBufferOpened => "initial_buffer_opened",
            Self::InitialSyntaxReady => "initial_syntax_ready",
            Self::AppReady => "app_ready",
            Self::TerminalEntered => "terminal_entered",
            Self::LspManagerSpawned => "lsp_manager_spawned",
            Self::FirstFramePresented => "first_frame_presented",
        }
    }
}

/// Ordered startup timings relative to entry into synchronous `main`.
///
/// The trace starts before the Tokio runtime is constructed, so optional
/// service startup cannot disappear into an unobserved wrapper interval.
#[cfg(feature = "startup-timing")]
#[derive(Debug)]
pub struct StartupTrace {
    origin: Instant,
    marks: Vec<(StartupPhase, Duration)>,
}

#[cfg(not(feature = "startup-timing"))]
#[derive(Debug, Default)]
pub struct StartupTrace;

#[cfg(feature = "startup-timing")]
impl Default for StartupTrace {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            marks: Vec::with_capacity(16),
        }
    }
}

impl StartupTrace {
    pub fn new() -> Self {
        #[cfg(feature = "startup-timing")]
        let mut trace = Self::default();
        #[cfg(not(feature = "startup-timing"))]
        let mut trace = Self;
        trace.mark(StartupPhase::MainEntry);
        trace
    }

    /// Records one milestone in a measurement build and disappears from a
    /// normal optimized build.
    #[cfg(feature = "startup-timing")]
    #[inline]
    pub fn mark(&mut self, phase: StartupPhase) {
        self.push(phase, self.origin.elapsed());
    }

    #[cfg(not(feature = "startup-timing"))]
    #[inline(always)]
    pub fn mark(&mut self, _phase: StartupPhase) {}

    /// Writes the report only when the measurement build was given an
    /// explicit destination. File output is safe while the alternate screen
    /// is active because it never writes diagnostic text to the terminal.
    #[cfg(feature = "startup-timing")]
    pub fn write_requested(&self) -> io::Result<Option<PathBuf>> {
        let Some(path) = env::var_os(STARTUP_TIMING_FILE_ENV).map(PathBuf::from) else {
            return Ok(None);
        };
        fs::write(&path, self.report())?;
        Ok(Some(path))
    }

    #[cfg(not(feature = "startup-timing"))]
    #[inline(always)]
    pub fn write_requested(&self) -> io::Result<Option<PathBuf>> {
        Ok(None)
    }

    #[cfg(feature = "startup-timing")]
    fn push(&mut self, phase: StartupPhase, elapsed: Duration) {
        debug_assert!(
            self.marks
                .last()
                .is_none_or(|(_, previous)| *previous <= elapsed),
            "startup milestones must be recorded in elapsed-time order"
        );
        self.marks.push((phase, elapsed));
    }

    #[cfg(feature = "startup-timing")]
    fn report(&self) -> String {
        let mut report = String::from("# Runyte startup timing\n");
        report.push_str("# durations are measured from synchronous main entry\n");
        for (phase, elapsed) in &self.marks {
            report.push_str(&format!(
                "{} {:.3} ms\n",
                phase.name(),
                elapsed.as_secs_f64() * 1_000.0
            ));
        }
        report
    }
}

#[cfg(all(test, feature = "startup-timing"))]
mod tests {
    use super::*;

    #[test]
    fn report_preserves_phase_order_and_elapsed_values() {
        let mut trace = StartupTrace {
            origin: Instant::now(),
            marks: Vec::new(),
        };
        trace.push(StartupPhase::MainEntry, Duration::ZERO);
        trace.push(StartupPhase::ConfigLoaded, Duration::from_micros(1_250));
        trace.push(StartupPhase::FirstFramePresented, Duration::from_millis(8));

        assert_eq!(
            trace.report(),
            "# Runyte startup timing\n\
             # durations are measured from synchronous main entry\n\
             main_entry 0.000 ms\n\
             config_loaded 1.250 ms\n\
             first_frame_presented 8.000 ms\n"
        );
    }
}

#[cfg(all(test, not(feature = "startup-timing")))]
mod disabled_tests {
    use super::*;

    #[test]
    fn the_default_trace_has_no_runtime_state() {
        assert_eq!(std::mem::size_of::<StartupTrace>(), 0);
        let mut trace = StartupTrace::new();
        trace.mark(StartupPhase::FirstFramePresented);
        assert_eq!(trace.write_requested().unwrap(), None);
    }
}
