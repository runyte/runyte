# Comparing Runyte with other tools

Runyte combines a modal editor with a terminal workspace. These comparisons
explain where that scope overlaps with other tools, where their choices differ,
and which workflows suit each. They are guides to choosing or combining tools,
not rankings.

## Comparisons

| Category | Tool | Main comparison |
| --- | --- | --- |
| Editors | [Helix](helix.md) | Shared selection-first roots; different workspace scope |
| Editors | [Neovim](neovim.md) | Integrated defaults and a customizable Vim-based environment |
| Editors | [Kakoune](kakoune.md) | Integrated workspace and composition with Unix tools |
| Editors | [Fresh](fresh.md) | Modal and conventional interaction in integrated terminal editors |
| Terminal workspaces | [Herdr](herdr.md) | Project editing and agent supervision |
| Terminal workspaces | [tmux](tmux.md) | Editor-owned workspace and general terminal multiplexing |
| Terminal workspaces | [Zellij](zellij.md) | Editor-owned workspace and terminal/plugin layouts |
| Graphical terminals | [tty7](tty7.md) | An editor inside a terminal and a graphical terminal workbench |

## Authoring template

Agents and human contributors should use the following structure for every
comparison. Name the file after the tool in lowercase, such as `helix.md`,
and add it to the index above. Keep the table central and the surrounding prose
short enough that readers can compare pages easily.

```markdown
# Runyte vs <Tool>

[All comparisons](README.md) · Last verified: YYYY-MM-DD

Scope: identify the Runyte checkout/release and the other tool's documentation
or release examined. State whether claims are documentation-based or tested.

## Summary

One or two short paragraphs: explain each tool's purpose, their meaningful
similarities, and the main distinction. Link the other tool's official overview.

## Feature comparison

Link the Runyte references used. Explain any assumptions that apply to the table.

| Area | Runyte | <Tool> |
| --- | --- | --- |
| Primary purpose | ... | ... |
| Editing | ... | ... |
| Terminals | ... | ... |
| Navigation and search | ... | ... |
| Persistence | ... | ... |
| Agent integration | ... | ... |
| Extensibility | ... | ... |
| Interaction model | ... | ... |

Add relevant rows after these, such as language tooling or remote work.
Attach official source links to the claims they support, in cells or immediately
adjacent prose. Explain significant qualifications below the table.

## Use cases

Present workflow recommendations as judgments based on the comparison.

- **Runyte:** A concrete situation where Runyte fits well.
- **<Tool>:** A concrete situation where the other tool fits well.
- **Together, migration, or evaluation:** A practical composition option,
  migration consideration, or useful way to evaluate both.
```

## Evidence and editorial rules

- Verify current official documentation or source before writing or refreshing
  a page. Record the actual verification date. Identify release-specific or
  development-only behavior explicitly; a live documentation page does not
  establish that every installed version has a feature.
- Use the common table rows in the same order, with Runyte first. Add only rows
  that help explain this particular comparison. Describe capabilities in prose
  rather than reducing different scopes to checkmarks.
- Separate built-in capabilities, bundled plugins, separately installed
  extensions, configuration, and external tools. Credit viable plugin workflows
  without presenting them as defaults. Never treat a roadmap as shipped behavior.
- Prefer primary sources: official manuals, repositories, release notes, and
  extension authors' documentation. Link near the supported claim; avoid bare
  URLs or a detached list of sources. Use repository-relative links for Runyte.
- Absence from an overview is not proof of absence. Check the relevant manual
  before asserting that a feature is missing. When evidence is limited, say
  what is documented or omit the claim.
- Distinguish live process retention during detach from layout restoration,
  unsaved-buffer recovery, and restarting or resuming agent conversations.
  Do not imply that processes survive a reboot.
- Give each tool a fair account of its strengths. Different design choices are
  not defects. Avoid unsupported claims about speed, memory, reliability,
  security, popularity, or ease of learning. Comparative measurements require
  reproducible methodology, versions, platform, and workload.
- Explain similarities as well as differences. Use cases should identify real
  needs rather than declare a universal winner. Label untested combinations as
  composition options, not validated integrations.
- Check Runyte claims against the [README](../../README.md), relevant
  [user-guide](../user-guide.md) sections, and current source. Read the
  [keymap register](../../context/reference/helix-keymap-v1.md) before discussing
  command compatibility; do not call Runyte Helix-compatible. Use the
  [UI vocabulary](../../context/reference/ui-vocabulary.md) for Runyte surfaces.
- Keep optional [plugins](../plugins.md) and the
  [context bridge](../../bridges/runyte-context/README.md) distinct. Describe
  bridge grants and terminal approval accurately without implying that agents
  can approve their own access or submit terminal commands automatically.
- Review related pages when shared Runyte behavior changes. Update the date only
  after rechecking the claims, not just when fixing spelling.

## Validation before handoff

Check that every page has the three prescribed sections, the common table rows,
an index backlink, a verification date, and working repository-relative links.
Review external sources for support and check table formatting and whitespace.
Documentation-only changes do not require Rust builds or coverage runs; follow
repository guidance if code changes are also part of the task.
