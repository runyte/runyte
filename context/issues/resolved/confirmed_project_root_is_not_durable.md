---
title: "A confirmed project directory is discarded, and in persistent mode the launch it was confirmed for fails"
status: resolved
reported: 2026-08-17
resolved: 2026-08-17
legacy_commit: 3ca377d
---

## Resolution

Commit `3ca377d` (`Make a confirmed project root outlast its prompt`) fixed two
independent losses of the same answer.

`project_root::prompt` returned the confirmed root and nothing else. The state
directory its question named was never created, by it or by anything
downstream: an editor session may write no runtime state at all, so the
directory that identifies a non-Git project simply never appeared. `discover`
recognizes such a project only by that directory, so every later launch asked
again. The prompt now calls `fs::create_dir_all` on the state root as part of
accepting the confirmation, after the `validate_state_root` check that already
ran earlier in the same iteration. The existing refusal text, `Runyte did not
create a project workspace directory`, had claimed this all along.

`workspace::lifecycle::start_detached_host` was the second loss. It passed the
child a working directory and a config path, but not the project root its
caller had just resolved, so the child ran `project_root::discover` a second
time. With neither marker present it fell through to `project_root::prompt` and
read EOF from the `Stdio::null()` stdin every detached host is spawned with,
bailing with `project directory was not confirmed`. The parent surfaced that as
`persistent workspace host exited with exit status: 1`, which is what ended the
launch. The child is now given `--project-root`, a new option carried on
`LaunchArguments`, and `main::run` skips discovery and the prompt whenever it is
present.

Both fixes are kept because they do not cover the same ground. Creating the
state directory would let the child rediscover the root on its own, but only
while `workspace.state` is a usable relative marker;
`project_root::is_usable_relative_marker` rejects an absolute path, so an
absolute `workspace.state` leaves nothing inside any project to find. Passing
the root covers that case and removes the child's dependence on re-deriving a
decision its parent had already made.

`main::resolve_requested_project_root` requires the launch directory to lie
inside the requested root, deliberately mirroring the containment
`start_detached_host` already enforces on the working directory it spawns in.
Workspace identity is derived from the root, so a root that does not contain the
launch directory would publish this process at another project's endpoint.
`--project-root` is refused in the modes that address a host by selector or list
every host, since those never resolve a project for it to name.

Making the answer durable also made its reach permanent, which the report had
raised as a condition on any fix. `discover` walks upward, so a state directory
confirmed at a home directory becomes the project root for every directory below
it lacking a marker of its own. This is supported on purpose —
`a_home_directory_can_own_a_project_workspace` covers it — so the fix warns
rather than refuses: `prompt` now names the consequence before asking to confirm
that one location. The home directory is passed in rather than read from the
environment inside `project_root`, matching the reserved roots beside it so no
test depends on the real home, and is canonicalized once on entry because
`$HOME` is commonly a symlink while every path compared against it has been
through `canonicalize`. `app::user_home_directory` became public so the launch
path uses the editor's answer instead of a second copy.

Coverage in `src/project_root.rs`:
`confirming_the_prompt_creates_the_state_directory_it_named`, which also asserts
that `discover` then resolves the same root;
`confirming_an_absolute_state_root_creates_it_outside_the_project`;
`prompt_accepts_the_home_directory_but_says_what_it_takes_in`;
`an_ordinary_directory_is_confirmed_without_the_home_warning`; and
`the_home_warning_survives_a_symlinked_home`. In `src/main.rs`:
`project_root_option_carries_a_resolved_workspace` and
`a_requested_project_root_must_contain_the_launch_directory`. In
`tests/persistent_host.rs`:
`detached_host_serves_a_project_it_could_not_have_discovered`, which starts a
real detached host in a project holding neither marker and fails on the old code
with the reported error. That test shuts its host down by explicit selector,
because a management command resolves its own project the same way a host would
have and is standing in a directory that cannot be resolved that way.

`tempfile` in `src/project_root.rs` gained an atomic counter in the same commit.
It keyed only on the clock, and the added tests were enough for two threads to
read one nanosecond and collide.

Known limitation: with an absolute `workspace.state` no marker is ever written
inside a project, so a management command such as `runyte --shutdown-host` given
no selector still cannot resolve the project from the current directory. That
configuration already makes every project share one state root, and was left as
a separate question.

## Report

Confirming a project directory at the non-Git prompt had no lasting effect, and
in persistent mode the launch it was given for failed outright.

In a directory that was neither a Git root nor held an existing `.runyte`,
`runyte` asked where its runtime state should live:

```
No Git repository or existing project workspace directory was found.
Project directory [/tmp/plain]:
Save Runyte project data in /tmp/plain/.runyte? [y/N]: y
```

Answering `y` returned the confirmed root to the caller and nothing else. The
directory the question named was never created. A `--standalone` launch opened
the editor normally, and the same directory listed afterwards still held only
its original files, so the next bare launch in it asked the same question again,
and every launch after that did too.

With `workspace.mode: persistent` the same answer ended the launch instead:

```
Save Runyte project data in /tmp/plain/.runyte? [y/N]: y
Error: persistent workspace host exited with exit status: 1: No Git repository
or existing project workspace directory was found.
Project directory [/tmp/plain]: Error: project directory was not confirmed;
Runyte did not create a project workspace directory
```

The second half of that message came from the detached host. A bare launch in
persistent mode resolved the project root, then spawned `runyte --serve` through
`workspace::lifecycle::start_detached_host`, which handed the child a working
directory and a config path but not the root that had just been confirmed. The
child ran `project_root::discover` again, found no Git root and no state
directory, fell through to `project_root::prompt`, and read EOF because the
child is spawned with `Stdio::null()` on stdin. It bailed with `project
directory was not confirmed`, and the parent reported the child's exit as a
startup failure.

The confirmation was therefore asked for on the terminal, accepted, and then
discarded twice over: once because nothing wrote it to disk, and again because
the process that needed it was never told. There was no workaround short of
`git init` or `--standalone`.

`runyte --wait` reaches `start_detached_host` by the same path and was exposed to
the same failure wherever it is used outside a Git checkout.

Creating the state directory on confirmation would fix the re-prompt and, by
leaving the marker `discover` looks for, would also let the child resolve the
root on its own — but only while `workspace.state` stays a usable relative
marker. `project_root::is_usable_relative_marker` rejects an absolute path, so a
configuration with an absolute `workspace.state` would still leave the child
with nothing to find. The child should not be re-deriving a decision its parent
has already made in any case.

The error text `Runyte did not create a project workspace directory` suggested
that confirming was meant to create one.

Whatever made the answer durable also had to account for how far one answer
reaches. `discover` walks upward, so a state directory confirmed at
`/home/user` becomes the project root for every directory below it that has no
Git repository and no state directory of its own — a single workspace covering
everything non-Git under home. Git roots are unaffected, since
`discover_candidates` checks every ancestor for one before it looks for a state
directory. A home directory owning a workspace is supported on purpose, and
`a_home_directory_can_own_a_project_workspace` in `src/project_root.rs` covers
it; what was new is that it became reachable by accepting a suggested default
twice.
