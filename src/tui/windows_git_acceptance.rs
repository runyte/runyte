// SPDX-License-Identifier: MPL-2.0

//! Exercise the real host startup with an isolated executable search path.
use super::*;
use runyte::test_support::TestRuntimeRoot;

#[test]
fn missing_git_disables_integration_at_host_startup() {
    use std::os::windows::process::CommandExt;
    let root = TestRuntimeRoot::new("windows-no-git-startup").unwrap();
    let log_path = root.join("fixture.log");
    let log = fs::File::create(&log_path).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "windows_git_acceptance::missing_git_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("PATH", root.path())
        .env("PATHEXT", ".EXE")
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("RUNYTE_CONTEXT_HOME", root.join("context"))
        .env("RUNYTE_GIT_TEST_ROOT", root.path())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "{}",
                fs::read_to_string(log_path).unwrap()
            );
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("missing-Git startup fixture timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "compiled subprocess fixture with isolated PATH and configuration"]
fn missing_git_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_GIT_TEST_ROOT").unwrap());
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME").unwrap(),
        root.join("config")
    );
    assert_eq!(
        std::env::var_os("RUNYTE_CONTEXT_HOME").unwrap(),
        root.join("context")
    );
    assert!(GitCliProvider::from_environment().is_none());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut config = Config::default();
        config.lsp.enable = false;
        let app = App::new_in_project(config, None, &root).unwrap();
        let mut host = WorkspaceHost::new(app);
        let services =
            start_host_services(&mut host, &mut StartupTrace::new(), None, false, None).unwrap();
        assert!(services.git_events.is_none());
        for command in ["git-status", "git-refresh", "git-branches", "git-worktrees"] {
            let spec = runyte::command::resolve_command(command).unwrap();
            assert_eq!(
                host.command_capabilities()
                    .command_availability(spec)
                    .reason(),
                Some("no `git` executable was found")
            );
            let outcome = host
                .app_mut()
                .execute(runyte::command::parse_colon_command(command).unwrap())
                .unwrap();
            assert!(
                matches!(outcome, runyte::app::CommandOutcome::UserError(ref reason)
                if reason == "no `git` executable was found")
            );
            assert!(host.status.contains("no `git` executable was found"));
        }
        assert!(!host.should_quit);
    });
    runtime.shutdown_timeout(Duration::from_secs(2));
}
