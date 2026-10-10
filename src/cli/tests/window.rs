// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    cli::{Edition, Environment},
    command::{
        CommandExecutionContext, CommandId, CommandInvocation, EditorCommand, InvocationParameters,
    },
    config::Config,
    startup::StartupTrace,
    test_support::TestRuntimeRoot,
};

struct Clipboard;
impl SystemClipboard for Clipboard {
    fn read(&mut self) -> anyhow::Result<String> {
        Ok("supplied clipboard".into())
    }
    fn write(&mut self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

struct ClipboardFrontend;
static FRONTEND: ClipboardFrontend = ClipboardFrontend;

impl WindowFrontend for ClipboardFrontend {
    fn system_clipboard(&self) -> Option<Box<dyn SystemClipboard>> {
        Some(Box::new(Clipboard))
    }
    fn launch(
        &self,
        _: Box<dyn FnOnce() -> anyhow::Result<()> + Send>,
        _: usize,
    ) -> anyhow::Result<()> {
        panic!("must not launch a window")
    }
    fn surface(&self) -> io::Result<Box<dyn WindowSurface>> {
        panic!("must not create a window surface")
    }
    fn events(&self) -> Box<dyn WindowEvents> {
        panic!("must not create window input")
    }
    fn dimensions(&self) -> (u16, u16) {
        panic!("must not query a window")
    }
    fn take_close_request(&self) -> bool {
        panic!("must not query a window")
    }
    fn begin_attachment(&self) {
        panic!("must not attach a window")
    }
    fn update_media(&self, _: &mut App) {
        panic!("must not update window media")
    }
    fn capture_media(&self, _: &HostFrame, _: &mut App, _: &KeyHintState) {
        panic!("must not capture window media")
    }
    fn render_frame(&self, _: &mut Frame<'_>, _: &App, _: &HostFrame, _: TerminalColorDepth) {
        panic!("must not render a window")
    }
    fn attached_media_requests(&self, _: &HostFrame) -> Vec<ClientRequest> {
        panic!("must not query window media")
    }
    fn receive_media_action(&self, _: FrameId, _: ViewRequest) {
        panic!("must not send window media")
    }
}

#[test]
fn desktop_host_and_editor_keep_supplied_clipboard_without_window_attachment() {
    let root = TestRuntimeRoot::new("edition-clipboard").unwrap();
    let path = root.join("note.txt");
    std::fs::write(&path, "").unwrap();
    for editor in [false, true] {
        let environment = Environment {
            edition: Edition::Desktop,
            window: Some(&FRONTEND),
        };
        let targets = vec![crate::launch::LaunchTarget::new(path.clone())];
        let mut startup = StartupTrace::new();
        let mut app = if editor {
            App::new_editor_with_frontend(
                Config::default(),
                targets,
                root.path(),
                &mut startup,
                environment,
            )
        } else {
            App::new_in_project_with_frontend(
                Config::default(),
                targets,
                root.path(),
                &mut startup,
                environment,
            )
        }
        .unwrap();
        app.note_loaded_config(&root.join("config.yaml"));
        assert_eq!(app.edition(), Edition::Desktop);
        let mut host = WorkspaceHost::new(app);
        host.app_mut()
            .execute(
                CommandInvocation::from_parts(
                    CommandId::Editor(EditorCommand::ClipboardPasteAfter),
                    InvocationParameters::None,
                    CommandExecutionContext::default(),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(host.app().active_buffer().to_string(), "supplied clipboard");
    }
}

#[test]
fn fixture_acceptance_rejects_zero_or_multiple_successful_tests() {
    crate::cli::assert_one_test_passed("test result: ok. 1 passed; 0 failed; 2 filtered out;\n");
    for output in [
        "test result: ok. 0 passed; 0 failed; 1 filtered out;",
        "test result: ok. 2 passed; 0 failed;",
        "test result: FAILED. 1 passed; 1 failed;",
        "test result: ok. 1 passed; 0 failed;\ntest result: ok. 0 passed; 0 failed;",
    ] {
        assert!(std::panic::catch_unwind(|| crate::cli::assert_one_test_passed(output)).is_err());
    }
}
