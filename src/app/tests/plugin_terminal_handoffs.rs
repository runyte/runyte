// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::plugin::application::{CapturedContext, ErrorCode};

struct Settled(std::sync::mpsc::Sender<()>);
impl Drop for Settled {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

fn context(app: &App) -> CapturedContext {
    CapturedContext {
        foreground_allowed: true,
        action: None,
        pane: app.active_pane,
        buffer: app.active().buffer,
        terminal: app.active().terminal,
        attachment: app.plugins.attachment_generation,
        foreground: app.plugins.foreground_generation,
    }
}
fn request() -> TerminalRequest {
    TerminalRequest {
        program: "/bin/sh".into(),
        arguments: vec!["-c".into(), "sleep 30".into()],
        directory: std::env::temp_dir(),
        label: "Plugin terminal".into(),
    }
}

#[test]
fn plugin_terminal_handoff_rechecks_foreground_and_preserves_native_pane_ownership() {
    let _guard = crate::terminal::pending_test_guard();
    let mut app = App::new(Config::default(), None).unwrap();
    let captured = context(&app);
    assert_eq!(
        app.reserve_plugin_terminal(request(), &captured)
            .unwrap_err()
            .code,
        ErrorCode::NoFrontend
    );
    app.note_plugin_frontend(true);
    for change in ["input", "attachment", "permission", "terminal"] {
        let captured = context(&app);
        let mut preparation = app.reserve_plugin_terminal(request(), &captured).unwrap();
        let (finished, completion) = std::sync::mpsc::channel();
        preparation.retain_until_settled(Box::new(Settled(finished)));
        let pending = preparation.spawn().unwrap();
        match change {
            "input" => app.plugins.foreground_generation += 1,
            "attachment" => {
                app.note_plugin_frontend(false);
                app.note_plugin_frontend(true);
            }
            "permission" => {}
            "terminal" => app.active_mut().terminal = Some(TerminalId::from_raw(u64::MAX)),
            _ => unreachable!(),
        }
        let mut captured = captured;
        if change == "permission" {
            captured.foreground_allowed = false;
        }
        assert_eq!(
            app.install_plugin_terminal(pending, &captured)
                .unwrap_err()
                .code,
            ErrorCode::ContextChanged,
            "{change}"
        );
        app.active_mut().terminal = None;
        assert!(app.terminals.iter().next().is_none());
        completion
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    }
    let captured = context(&app);
    let original = app.active().buffer;
    let preparation = app.reserve_plugin_terminal(request(), &captured).unwrap();
    let cancellation = preparation.cancellation();
    let pending = preparation.spawn().unwrap();
    let id = app.install_plugin_terminal(pending, &captured).unwrap();
    assert_eq!(app.active_terminal(), Some(id));
    assert_eq!(app.active().buffer, original);
    assert!(app.terminals.get(id).unwrap().live());
    assert_eq!(app.mode, Mode::Insert);
    cancellation.cancel();
    assert_eq!(app.active_terminal(), Some(id));
    app.leave_terminal();
    assert_eq!(app.active_terminal(), None);
    assert_eq!(app.active().buffer, original);
    assert!(app.terminals.get(id).unwrap().live());
}
