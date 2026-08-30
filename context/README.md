# Development context

This directory holds the durable development record for Runyte. It is kept
separate from the user guide and from runtime workspace state.

- `issues/` contains open work, deferred problems, and resolved diagnoses.
- `plans/` contains approved, proposed, completed, and superseded development
  plans according to the lifecycle described in `plans/README.md`.
- `reference/` contains current registers of record for behavior or procedures
  that must remain consistent across code and documentation.
- `reviews/` contains findings about work in progress: reviews of a branch
  before it lands, and investigations whose evidence has to outlive the
  session that gathered it.

Files directly under `issues/` are open. `issues/deferred/` contains confirmed
problems that need a broader design decision before implementation.
`issues/resolved/` preserves diagnoses, deliberate limitations, and regression
tests for completed work; it is searched as needed rather than read wholesale.

A file under `reviews/` is named after its subject in `snake_case` and states
what was examined, at which commit, and what was verified rather than assumed,
so that somebody working on another machine or another platform can act on it
without repeating the work. Reviews are transient: once a branch lands or an
investigation resolves, whatever is durable belongs in an `issues/resolved/`
record or a `reference/` register, and the review is removed. A review is not
a second source of current behavior.

Development context is written as neutral technical prose. It records observed
behavior, decisions, constraints, and evidence rather than conversations or
the prompts that initiated the work. Personal paths, credentials, private
reasoning, unrestricted tool output, and editor runtime state do not belong
here.

The optional persistent-session host stores runtime state under the ignored
`.runyte/` directory. Runtime state never belongs under `context/`.

## Public history boundary

The public repository begins from a cleaned source snapshot. Resolved records
that predate that snapshot retain a `legacy_commit` identifier as provenance
from private development history; those objects are not part of the public Git
graph. Their diagnoses, limitations, and named regression tests are
self-contained. Issues resolved after the public history begins use reachable
public `commit` identifiers instead.
