// SPDX-License-Identifier: MPL-2.0

//! Runyte is an editor-first, agent-ready terminal workspace for software
//! development. It combines modal text editing, file management, terminal
//! multiplexing, a Git interface with worktree support, and persistent sessions
//! with detachable clients.

pub mod about;
pub mod app;
pub mod buffer;
pub mod clipboard;
pub mod command;
pub mod config;
pub mod content_alignment;
pub mod diff;
pub mod diff_view;
pub mod directory_buffer;
pub mod directory_listing;
pub mod external_open;
pub mod file_monitor;
pub mod file_picker;
pub mod finder;
pub mod fs_plan;
pub mod git;
pub mod git_monitor;
pub mod hash;
pub mod headless;
pub mod help;
pub(crate) mod help_document;
pub mod input;
pub mod input_grammar;
pub mod jump_labels;
pub mod jumplist;
pub mod key_hints;
pub mod keymap;
pub mod launch;
pub mod layout;
pub mod log;
pub mod lsp;
pub mod manual;
pub mod notification;
pub mod path_safety;
pub mod picker;
#[cfg(unix)]
pub mod process_group;
pub mod project_root;
#[cfg(unix)]
pub mod protocol;
pub mod row_hints;
pub mod selection;
pub mod service_health;
pub mod settings;
pub mod snapshot;
pub mod startup;
mod structural_selection;
pub mod syntax;
pub mod table;
pub mod terminal;
pub mod text;
pub mod tui;
pub mod tutorial;
pub mod ui;
#[cfg(unix)]
pub(crate) mod user_paths;
pub mod word_index;
pub mod workspace;
pub mod wrap;
