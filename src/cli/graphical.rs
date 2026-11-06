// SPDX-License-Identifier: MPL-2.0
//! Graphical launch preparation and admission; terminal launch rules stay in cli.
use super::*;

pub(super) fn prepare_targets(
    arguments: &mut LaunchArguments,
    directory: &Path,
    workspace: bool,
) -> Result<PathBuf> {
    if !arguments.graphical_launch || arguments.targets.is_empty() {
        return Ok(directory.to_owned());
    }
    let directory = arguments
        .graphical_directory
        .as_deref()
        .unwrap_or(directory);
    for target in &mut arguments.targets {
        if target.path.is_relative() {
            target.path = directory.join(&target.path);
        }
    }
    let request = crate::protocol::ClientRequest::OpenLaunchTargets {
        targets: arguments.targets.iter().cloned().map(Into::into).collect(),
    };
    request.validate().map_err(anyhow::Error::msg)?;
    if !workspace {
        return Ok(directory.to_owned());
    }
    let first = &arguments.targets[0].path;
    let identity = first.canonicalize().unwrap_or_else(|_| first.clone());
    let workspace = if identity.is_dir() {
        identity.as_path()
    } else {
        identity
            .parent()
            .context("graphical file has no directory")?
    };
    workspace
        .canonicalize()
        .context("cannot resolve the first graphical target's directory")
}

#[cfg(unix)]
pub(super) fn open_targets(
    host: &mut WorkspaceHost,
    targets: Vec<crate::protocol::LaunchTarget>,
    initial: bool,
) -> Result<()> {
    anyhow::ensure!(
        initial && host.app().native_media,
        "launch targets require the initial native interactive attachment"
    );
    crate::protocol::ClientRequest::OpenLaunchTargets {
        targets: targets.clone(),
    }
    .validate()
    .map_err(anyhow::Error::msg)?;
    let targets = targets
        .into_iter()
        .map(TryInto::try_into)
        .collect::<std::result::Result<Vec<_>, String>>()
        .map_err(anyhow::Error::msg)?;
    host.app_mut().open_native_targets(targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        launch::{LaunchPosition, LaunchTarget},
        test_support::TestRuntimeRoot,
    };

    #[test]
    fn only_marked_graphical_targets_choose_a_directory_and_become_absolute() {
        let root = TestRuntimeRoot::new("graphical-directory").unwrap();
        let launch = root.join("launch");
        let project = root.join("project");
        fs::create_dir(&launch).unwrap();
        fs::create_dir(&project).unwrap();
        let file = project.join("notes.txt");
        fs::write(&file, "notes").unwrap();
        let mut arguments =
            LaunchArguments::parse_from(["../project/notes.txt".into(), "other.txt".into()])
                .unwrap();
        assert_eq!(
            prepare_targets(&mut arguments, &launch, true).unwrap(),
            launch
        );
        assert!(arguments.targets[0].path.is_relative());
        arguments.window = true;
        arguments.graphical_launch = true;
        assert_eq!(
            prepare_targets(&mut arguments, &launch, true).unwrap(),
            project.canonicalize().unwrap()
        );
        assert_eq!(
            arguments.targets[0].path,
            launch.join("../project/notes.txt")
        );
        assert_eq!(arguments.targets[1].path, launch.join("other.txt"));
        arguments.targets = vec![LaunchTarget::new(&project)];
        assert_eq!(
            prepare_targets(&mut arguments, &launch, true).unwrap(),
            project.canonicalize().unwrap()
        );
        arguments.targets = vec![LaunchTarget::new(project.join("new.txt"))];
        assert_eq!(
            prepare_targets(&mut arguments, &launch, true).unwrap(),
            project.canonicalize().unwrap()
        );
        arguments.targets = vec![LaunchTarget::new(&file); crate::protocol::MAX_PATHS + 1];
        assert!(prepare_targets(&mut arguments, &launch, true).is_err());
        arguments.targets.clear();
        assert_eq!(
            prepare_targets(&mut arguments, &launch, true).unwrap(),
            launch
        );
    }

    #[test]
    fn bundle_home_cwd_does_not_change_original_relative_file_meaning() {
        let root = TestRuntimeRoot::new("bundle-cwd").unwrap();
        let original = root.create_private_dir("original").unwrap();
        let home = root.create_private_dir("home").unwrap();
        let path = original.join("notes.txt");
        fs::write(&path, "notes").unwrap();
        let mut arguments = LaunchArguments::parse_from([
            "--window".into(),
            "--graphical-launch".into(),
            "--graphical-cwd".into(),
            original.clone().into_os_string(),
            "notes.txt".into(),
        ])
        .unwrap();
        assert_eq!(
            prepare_targets(&mut arguments, &home, true).unwrap(),
            original
        );
        assert_eq!(arguments.targets[0].path, path);
    }

    #[test]
    fn editor_mode_can_open_a_new_file_without_a_workspace_directory() {
        let root = TestRuntimeRoot::new("graphical-editor-new-file").unwrap();
        let mut arguments = LaunchArguments::parse_from(
            [
                "--window",
                "--graphical-launch",
                "--editor",
                "missing-parent/new.txt",
            ]
            .map(Into::into),
        )
        .unwrap();
        assert_eq!(
            prepare_targets(&mut arguments, root.path(), false).unwrap(),
            root.path()
        );
        assert_eq!(
            arguments.targets[0].path,
            root.join("missing-parent/new.txt")
        );
        assert!(!root.join("missing-parent").exists());
    }

    #[test]
    fn graphical_mode_honors_configuration_but_terminal_targets_stay_standalone() {
        for args in [
            vec!["--window", "--graphical-launch"],
            vec!["--window", "--graphical-launch", "+2:3", "notes.txt"],
        ] {
            let arguments = LaunchArguments::parse_from(args.into_iter().map(Into::into)).unwrap();
            assert!(uses_automatic_persistent_mode(&arguments, RunMode::Mux));
            assert!(launch_is_editor(&arguments, RunMode::Editor));
            assert!(!launch_is_editor(&arguments, RunMode::Ide));
        }
        let arguments =
            LaunchArguments::parse_from(["--window".into(), "notes.txt".into()]).unwrap();
        assert!(!uses_automatic_persistent_mode(&arguments, RunMode::Mux));
    }

    #[cfg(unix)]
    #[test]
    fn launch_admission_validates_whole_payload_and_preserves_caret_policy() {
        let root = TestRuntimeRoot::new("graphical-admission").unwrap();
        let path = root.join("notes.txt");
        fs::write(&path, "one\n文 abc\nthree").unwrap();
        let mut app = App::new_in_project(Config::default(), None, &root).unwrap();
        app.note_loaded_config(&root.join("config.yaml"));
        app.programs = external_open::ProgramCache::load(Some(root.join("program-cache")));
        let mut host = WorkspaceHost::new(app);
        let target = LaunchTarget::at(
            &path,
            LaunchPosition {
                line: std::num::NonZeroUsize::new(2).unwrap(),
                column: std::num::NonZeroUsize::new(3),
            },
        );
        let wire = crate::protocol::LaunchTarget::from(target);
        assert!(open_targets(&mut host, vec![wire.clone()], true).is_err());
        host.app_mut().native_media = true;
        assert!(open_targets(&mut host, vec![wire.clone()], false).is_err());
        let mut bad = wire.clone();
        bad.line = Some(0);
        assert!(open_targets(&mut host, vec![wire.clone(), bad], true).is_err());
        assert!(
            host.app()
                .buffers
                .iter()
                .all(|buffer| buffer.path.as_deref() != Some(path.as_path()))
        );
        open_targets(&mut host, vec![LaunchTarget::new(&path).into(), wire], true).unwrap();
        assert_eq!(host.app().active_buffer().path.as_ref(), Some(&path));
        assert_eq!(
            host.app().cursor_position(),
            crate::buffer::Position::new(1, 2)
        );
    }
}
