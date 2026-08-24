---
title: "Duplicate key bindings make the command surface harder to learn"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: 884e13b
---

## Resolution

Commit `884e13b` (`Simplify duplicate key bindings`) corrected `built_in_bindings` in `src/keymap.rs`, where canonical application namespaces and historical short or compatibility aliases had accumulated for the same commands. It removes the `Space F` namespace, `Space :`, the short clipboard and language aliases, and the quit-shaped pane-close aliases while retaining the requested short file actions, canonical `Space c` and `Space l` namespaces, LSP goto paths, and window compatibility sequences.

Save remains one semantic command whether invoked with `Ctrl-s`, `:write`, or `:w`. System clipboard actions remain distinct from bare `y/p/P`: `Space c y/p/P` uses the operating-system clipboard, while the bare forms use Runyte's internal register. The Insert registry now keeps only Backspace, Delete, and Alt-Backspace for the requested existing deletion actions. Alt-Delete invokes the new `delete-word-forward` command, whose word-class traversal handles Unicode and line boundaries transactionally.

The same registry continues to drive execution, help, and key hints. The deliberate compatibility boundary is that requested `Ctrl-w` window sequences remain available, including their control-key and arrow suffix variants, while ordinary Insert-mode `Ctrl-w` is no longer a delete-word binding.

Tests covering the behavior are `removed_duplicate_bindings_stay_unbound`, `nested_space_tree_is_exact_primary_and_keeps_fast_compatibility_paths`, and `select_and_insert_modes_share_the_registry_without_duplicate_dispatch_tables` in `tests/keymap.rs`; `alt_delete_deletes_forward_by_word_class_across_unicode_and_lines` and `insert_mode_word_and_line_deletion_bindings_edit_without_literal_input` in `src/app.rs`; and the namespace presentation tests in `tests/key_hints.rs`.

Known limitation: terminal emulators must report Alt-Delete as an Alt-modified Delete event for that binding to be distinguishable.

## Report

Duplicate keybindings to be removed.

### Space/application aliases

Action                             Bindings
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  ━━━━━━━━━━━━━━━━━━━━
Open explorer                      Space F e, Space e  -> keep only Space e
─────────────────────────────────  ────────────────────
Open working-directory explorer    Space F E, Space E  -> keep only Space E
─────────────────────────────────  ────────────────────
Open file picker                   Space F f, Space f  -> keep only Space f
─────────────────────────────────  ────────────────────
Open buffer picker                 Space F b, Space b  -> keep only Space b
─────────────────────────────────  ────────────────────
Workspace search                   Space F /, Space /  -> keep only Space /
─────────────────────────────────  ────────────────────
Save                               Space F s, Ctrl-s   -> open question: same as :write and :w ?

The conclusion from the above is that the Space F namespace can be removed.

─────────────────────────────────  ────────────────────
Command palette                    Space :, :          -> Keep only :
─────────────────────────────────  ────────────────────

Clipboard yank                     Space c y, Space y  -> open question: does it differ from "y" alone?
─────────────────────────────────  ────────────────────
Clipboard paste after              Space c p, Space p  -> open question: does it differ from "p" alone?
─────────────────────────────────  ────────────────────
Clipboard paste before             Space c P, Space P  -> open question: does it differ from "P" alone?
─────────────────────────────────  ────────────────────
Documentation                      Space l h, Space h -> keep only Space l variant
─────────────────────────────────  ────────────────────
Document symbols                   Space l s, Space s -> keep only Space l variant
─────────────────────────────────  ────────────────────
Workspace symbols                  Space l S, Space S -> keep only Space l variant
─────────────────────────────────  ────────────────────
Diagnostics                        Space l d, Space d -> keep only Space l variant
─────────────────────────────────  ────────────────────
Rename symbol                      Space l r, Space r -> keep only Space l variant
─────────────────────────────────  ────────────────────
Code action                        Space l a, Space a -> keep only Space l variant
─────────────────────────────────  ────────────────────
Definition                         Space l g d, g d   -> keep both
─────────────────────────────────  ────────────────────
Declaration                        Space l g D, g D   -> keep both
─────────────────────────────────  ────────────────────
Type definition                    Space l g y, g y   -> keep both
─────────────────────────────────  ────────────────────
References                         Space l g r, g r   -> keep both
─────────────────────────────────  ────────────────────
Implementation                     Space l g i, g i   -> keep both

### Windows and splits

Action              Bindings
━━━━━━━━━━━━━━━━━━  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Close window        Space w q, Space w c, Ctrl-w q, Ctrl-w c, Ctrl-w Ctrl-q  -> remove Space w q, Ctrl-w q, Ctrl-w Ctrl-q
──────────────────  ─────────────────────────────────────────────────────────
Next window         Space w w, Ctrl-w w, Ctrl-w Ctrl-w -> keep all
──────────────────  ─────────────────────────────────────────────────────────
Only window         Space w o, Ctrl-w o, Ctrl-w Ctrl-o -> keep all
──────────────────  ─────────────────────────────────────────────────────────
Focus left          Space w h, Ctrl-w h, Ctrl-w Ctrl-h, Ctrl-w Left -> keep all
──────────────────  ─────────────────────────────────────────────────────────
Focus down          Space w j, Ctrl-w j, Ctrl-w Ctrl-j, Ctrl-w Down -> keep all
──────────────────  ─────────────────────────────────────────────────────────
Focus up            Space w k, Ctrl-w k, Ctrl-w Ctrl-k, Ctrl-w Up -> keep all
──────────────────  ─────────────────────────────────────────────────────────
Focus right         Space w l, Ctrl-w l, Ctrl-w Ctrl-l, Ctrl-w Right -> keep all
──────────────────  ─────────────────────────────────────────────────────────
Vertical split      Space w v, Ctrl-w v, Ctrl-w Ctrl-v -> keep all
──────────────────  ─────────────────────────────────────────────────────────
Horizontal split    Space w s, Ctrl-w s, Ctrl-w Ctrl-s -> keep all

The intent is that "quit" exits the entire application and "close" closes panes and buffers.

Inside an explorer, the split sequences are contextually overridden:

Explorer action                   Bindings
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Open entry in vertical split      Space w v, Ctrl-w v, Ctrl-w Ctrl-v -> keep all
────────────────────────────────  ────────────────────────────────────
Open entry in horizontal split    Space w s, Ctrl-w s, Ctrl-w Ctrl-s -> keep all
────────────────────────────────  ────────────────────────────────────
Open parent directory             -, Backspace -> keep all

### Movement and editing

Action                            Bindings
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Move left                         h, Left
────────────────────────────────  ────────────────────────────────────
Move right                        l, Right
────────────────────────────────  ────────────────────────────────────
Move down                         j, Down
────────────────────────────────  ────────────────────────────────────
Move up                           k, Up
────────────────────────────────  ────────────────────────────────────
Line start                        0, g h, Home
────────────────────────────────  ────────────────────────────────────
Line end                          $, g l, End
────────────────────────────────  ────────────────────────────────────
File end                          G, g e
────────────────────────────────  ────────────────────────────────────
Jump forward                      Ctrl-i, Tab
────────────────────────────────  ────────────────────────────────────
Split selection on newline        S, Alt-s
────────────────────────────────  ────────────────────────────────────
Insert newline                    Enter, Ctrl-j
────────────────────────────────  ────────────────────────────────────
Delete backward in Insert         Backspace, Shift-Backspace, Ctrl-h -> keep only backspace
────────────────────────────────  ────────────────────────────────────
Delete forward in Insert          Delete, Ctrl-d -> keep only delete
────────────────────────────────  ────────────────────────────────────
Delete word backward in Insert    Ctrl-w, Alt-Backspace -> keep Alt-Backspace.

Alt-Delete should delete the next word in Insert mode.
