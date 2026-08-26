# Uniform theme construction

Status: completed 2026-08-26

Created: 2026-08-26

## Decision

Runyte keeps one resolved `Theme` contract and one frontend rendering path.
Built-in palette families may retain the construction mechanism that best
expresses their upstream source, shared role mapping, and deliberate Runyte
adaptations; they do not have to be flattened into repetitive literals.

Every built-in family exposes the same registration interface:

```rust
fn themes() -> impl Iterator<Item = (String, ThemeDefinition)>
```

`Config::default` combines those iterators without knowing how a family
constructs its definitions. Standalone definitions, shared family palettes,
and explicitly derived variants therefore remain visible at their natural
ownership boundary while all of them enter the same resolver.

## Source organization

- Move Runyte's original standalone palettes into a core-theme module.
- Keep Catppuccin's four flavours behind their shared role adapter.
- Keep Everforest's background variants behind their shared foreground and
  role adapter.
- Keep Nightfox-derived palettes and Runyte's explicit variants together.
- Keep Zenbones' pinned generated palette data in its existing dedicated
  module.
- Leave `ThemeDefinition`, `Theme`, color parsing, fallbacks, appearance, and
  derived grounds in `src/config.rs` as the common configuration contract.

Each module returns definitions only. Theme lookup, validation, fallback
resolution, persistence, snapshots, protocol conversion, and rendering remain
family-independent.

## Implemented result

The core, Catppuccin, Everforest, Nightfox, standalone-import, and Zenbones
modules now each expose `themes()`, while `Config::default` sees only their
combined registry. `src/config.rs` retains the shared definition, resolution,
fallback, and configuration behavior.

The status-only colors remain in definitions, resolved themes, and protocol
frames for compatibility with the crate's public Rust and serialized shapes.
They remain compatibility data only: bundled frontends continue to render the
global status line with the ordinary theme foreground and background.

Regression coverage is in `src/config.rs` through
`config::tests::family_registrations_are_disjoint_and_cover_the_built_in_inventory`
and
`config::tests::compatibility_status_theme_keys_remain_resolved`,
in `src/ui.rs` through
`ui::tests::resolved_theme_roles_reach_the_frontend_adapter`, and in
`src/protocol/mod.rs` through the complete theme equality check in
`protocol::tests::notification_frame_values_and_ui_vocabulary_round_trip_over_v11`.

## Resolved contract

Tests resolve the full built-in inventory rather than validating only source
definitions. The contract requires every built-in to resolve, every semantic
syntax scope to be populated, and the complete resolved value to survive the
private protocol round trip.

The global status line follows `context/reference/ui-vocabulary.md`: outside
the mode label it uses the ordinary theme foreground and background. The old
`status_foreground` and `status_background` fields have no renderer role, but
remain in public definitions, resolved themes, and protocol frames so this
internal reorganization does not also become a Rust or serialized-data
compatibility break.

## Verification

Focused tests cover stable theme registration, custom-theme fallbacks, syntax
scope completeness, appearance-derived grounds, and complete protocol
round-tripping. Completion verification passed with:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
