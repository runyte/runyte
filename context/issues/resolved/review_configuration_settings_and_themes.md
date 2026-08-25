---
title: "Configuration paths, interval bounds, and failed previews were not consistently hardened"
status: resolved
reported: 2026-08-25
resolved: 2026-08-25
commit: cdb306b
---

## Resolution

Commit `cdb306b` (`Harden configuration and settings handling`) resolved the
review. `Config::load` was returning a caller-supplied relative path after
parsing it, so workspace initialization could change the meaning of that path
before settings or persistent-session lifecycle code reused it. The loader now
anchors relative paths to the launch directory once, keeps absolute paths usable
even when the current directory has disappeared, and every lifecycle path that
loads configuration carries that resolved identity forward. The loader also
used `Path::exists`, which made a dangling configuration symlink indistinguishable
from a missing file; it now inspects the path identity and reports the broken
link without replacing it.

`Config::validate_settings` was missing the registry's upper bounds for
`workspace.idle_retirement_minutes` and `git.refresh_interval_seconds`, allowing
startup YAML to bypass limits enforced by the settings picker. The bounds now
live beside the configuration model and are shared by loading and setting
metadata. Finally, `App::persist_selected_setting` left an immediate preview
active when no configuration path was available or lossless persistence failed.
It now restores the complete runtime preview snapshot on every save failure,
keeps a retryable choice popup open where no write occurred, and reports the
recovery. The defensive post-write apply-error path also restores runtime state
and explains that the saved value requires a restart.

The broader audit confirmed that the existing lossless scalar patcher rereads
the source immediately before editing, preserves comments, ordering, unknown
fields, file modes, and authored symlinks, and atomically replaces ordinary
files. Existing theme fallback, missing-color derivation, and contrast tests
cover the theme invariants, so no theme implementation change was required.

Regression coverage is in `src/config.rs`:
`config::tests::a_filename_only_config_path_resolves_from_the_launch_directory`,
`config::tests::absolute_config_load_does_not_require_a_live_cwd`,
`config::tests::loading_a_dangling_config_symlink_reports_the_broken_identity`,
and
`config::tests::loading_enforces_every_registry_backed_runtime_interval_bound`.
Preview recovery is covered in `src/app/tests/presentation_and_settings.rs` by
`app::tests::presentation_and_settings::failed_setting_write_keeps_the_picker_but_rolls_back_its_live_preview`
and
`app::tests::presentation_and_settings::missing_config_path_refuses_the_save_and_rolls_back_its_live_preview`.
The preservation contract remains covered in `src/settings.rs`, including
`settings::tests::replacing_a_scalar_preserves_every_other_byte` and
`settings::tests::a_setting_write_follows_a_symlink_without_replacing_it`.

Known limitation: the atomic writer rereads the latest file immediately before
patching it, but portable filesystems provide no compare-and-swap primitive that
can prevent an external writer from racing specifically between that read and
the final rename.

## Report

Configuration discovery, YAML parsing and patching, setting metadata, live
previews, persistence, and theme resolution required a focused hardening review.
The review was proactive rather than prompted by a previously reproduced defect;
only confirmed problems were eligible for change, and every confirmed problem
that was safely local to this category needed resolution.

The primary scope was `src/config.rs`, `src/config/`, `src/settings.rs`,
`src/app/settings_workflows.rs`, configuration presentation, and their tests.
The review covered defaults and aliases, unknown and malformed values, numeric
bounds, duplicate or conflicting YAML, lossless patch refusal, atomic writes,
permission and external-change handling, restart-required settings, preview
rollback, custom themes, missing colors and fallback chains,
contrast-sensitive derived colors, configuration-path identity, and isolation
from real user configuration during tests.

Confirmed defects were:

- Relative configuration paths retained their relative spelling after load and
  could resolve to a different file after a later working-directory change.
  Persistent-session lifecycle commands also discarded the normalized path
  returned by the loader.
- Loading an absolute configuration path unnecessarily depended on a usable
  current working directory, while a dangling configuration symlink was treated
  as though no configuration file existed.
- Startup YAML did not enforce the settings registry's maximum values of 43,200
  for `workspace.idle_retirement_minutes` and 3,600 for
  `git.refresh_interval_seconds`.
- When saving an immediate setting preview failed, including when no loaded
  configuration path existed, the previewed runtime value remained active until
  a separate cancellation action.

Regression tests use temporary paths and do not access real user configuration
or cache locations. Configuration patching continues to preserve comments,
ordering, and unknown YAML fields wherever the documented lossless-patching
contract promises that behavior.
