// SPDX-License-Identifier: MPL-2.0

use std::ops::Deref;

use crate::{
    app::{App, CompletionSource, FrameGeometry, MaximizedView, Mode, PromptKind},
    config::{Color as RunyteColor, Theme},
    diff::Change,
    git::{CountKind, DiffLine, LineChange},
    input::KeyCode,
    key_hints::{KeyHintRow, KeyHintState},
    keymap::BindingAvailability,
    layout::Rect,
    snapshot::{
        EditorSnapshot, OverlayKind, OverlayLayout, OverlayPreview, OverlaySnapshot, PaneSnapshot,
        SnapshotRow, StatusSnapshot, TextRole, TextRunKind,
    },
    terminal::{Cell as TerminalCell, TerminalView},
    workspace::HostFrame,
};
use ratatui::{
    Frame,
    layout::{
        Constraint, Direction, Layout as TuiLayout, Position as ScreenPosition, Rect as TuiRect,
    },
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, StatefulWidget,
        Wrap,
    },
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::notification::{NotificationCounts, NotificationSeverity};

/// Ratatui's resolved copy of a Runyte theme.
///
/// Conversion happens once at the start of a frame. Everything below this
/// adapter uses terminal colors, while `App` and `Config` retain only
/// frontend-independent values.
struct TuiTheme {
    background: ratatui::style::Color,
    /// The ground behind panes that do not own input, halfway toward the
    /// overlay ground so the three layers remain visually ordered.
    inactive_background: ratatui::style::Color,
    /// The ground every floating overlay paints on, one step off
    /// `background`. Derived from the theme rather than sent with it, so an
    /// attached client separates a host's overlays the same way a standalone
    /// session separates its own.
    overlay_background: ratatui::style::Color,
    foreground: ratatui::style::Color,
    muted: ratatui::style::Color,
    jump_text_muted: ratatui::style::Color,
    accent: ratatui::style::Color,
    cursor_normal: ratatui::style::Color,
    cursor_insert: ratatui::style::Color,
    cursor_select: ratatui::style::Color,
    cursor_command: ratatui::style::Color,
    directory: ratatui::style::Color,
    selection: ratatui::style::Color,
    selection_primary: ratatui::style::Color,
    fuzzy_match_secondary: ratatui::style::Color,
    fuzzy_match_primary: ratatui::style::Color,
    error: ratatui::style::Color,
    warning: ratatui::style::Color,
    info: ratatui::style::Color,
    jump_label_immediate: ratatui::style::Color,
    jump_label_primary: ratatui::style::Color,
    jump_label_secondary: ratatui::style::Color,
    change_added: ratatui::style::Color,
    change_modified: ratatui::style::Color,
    change_removed: ratatui::style::Color,
    diff_added: Option<ratatui::style::Color>,
    diff_removed: Option<ratatui::style::Color>,
    diff_changed: Option<ratatui::style::Color>,
    syntax: Vec<Option<ratatui::style::Color>>,
}

impl TuiTheme {
    fn new(theme: &Theme) -> Self {
        Self {
            background: to_tui_color(theme.background),
            inactive_background: to_tui_color(theme.inactive_background()),
            overlay_background: to_tui_color(theme.overlay_background()),
            foreground: to_tui_color(theme.foreground),
            muted: to_tui_color(theme.muted),
            jump_text_muted: to_tui_color(theme.jump_text_muted),
            accent: to_tui_color(theme.accent),
            cursor_normal: to_tui_color(theme.cursor_normal),
            cursor_insert: to_tui_color(theme.cursor_insert),
            cursor_select: to_tui_color(theme.cursor_select),
            cursor_command: to_tui_color(theme.cursor_command),
            directory: to_tui_color(theme.directory),
            selection: to_tui_color(theme.selection),
            selection_primary: to_tui_color(theme.selection_primary),
            fuzzy_match_secondary: to_tui_color(theme.fuzzy_match_secondary),
            fuzzy_match_primary: to_tui_color(theme.fuzzy_match_primary),
            error: to_tui_color(theme.error),
            warning: to_tui_color(theme.warning),
            info: to_tui_color(theme.info),
            jump_label_immediate: to_tui_color(theme.jump_label_immediate),
            jump_label_primary: to_tui_color(theme.jump_label_primary),
            jump_label_secondary: to_tui_color(theme.jump_label_secondary),
            change_added: to_tui_color(theme.change_added),
            change_modified: to_tui_color(theme.change_modified),
            change_removed: to_tui_color(theme.change_removed),
            diff_added: theme.diff_added.map(to_tui_color),
            diff_removed: theme.diff_removed.map(to_tui_color),
            diff_changed: theme.diff_changed.map(to_tui_color),
            syntax: theme
                .syntax
                .iter()
                .map(|color| color.map(to_tui_color))
                .collect(),
        }
    }

    fn syntax_color(&self, scope: crate::syntax::Scope) -> Option<ratatui::style::Color> {
        self.syntax.get(scope.index()).copied().flatten()
    }

    fn cursor(&self, mode: Mode) -> ratatui::style::Color {
        match mode {
            Mode::Insert => self.cursor_insert,
            Mode::Select => self.cursor_select,
            Mode::Command => self.cursor_command,
            Mode::Normal => self.cursor_normal,
        }
    }

    fn pane_background(&self, active: bool) -> ratatui::style::Color {
        if active {
            self.background
        } else {
            self.inactive_background
        }
    }

    fn status_style(&self) -> Style {
        Style::default().fg(self.foreground).bg(self.background)
    }

    fn mode_status_style(&self, mode: Mode) -> Style {
        Style::default().fg(self.background).bg(self.cursor(mode))
    }
}

fn to_tui_color(color: RunyteColor) -> ratatui::style::Color {
    use ratatui::style::Color as TuiColor;

    match color {
        RunyteColor::Reset => TuiColor::Reset,
        RunyteColor::Black => TuiColor::Black,
        RunyteColor::Red => TuiColor::Red,
        RunyteColor::Green => TuiColor::Green,
        RunyteColor::Yellow => TuiColor::Yellow,
        RunyteColor::Blue => TuiColor::Blue,
        RunyteColor::Magenta => TuiColor::Magenta,
        RunyteColor::Cyan => TuiColor::Cyan,
        RunyteColor::White => TuiColor::White,
        RunyteColor::Gray => TuiColor::Gray,
        RunyteColor::DarkGray => TuiColor::DarkGray,
        RunyteColor::Rgb(red, green, blue) => TuiColor::Rgb(red, green, blue),
    }
}

/// Frame-local adapter that shadows `App::theme` with its converted palette
/// and otherwise delegates to the application facade.
struct TuiApp<'a> {
    app: &'a App,
    theme: TuiTheme,
}

impl<'a> TuiApp<'a> {
    fn new(app: &'a App, theme: &Theme) -> Self {
        let theme = TuiTheme::new(theme);
        Self { app, theme }
    }
}

impl Deref for TuiApp<'_> {
    type Target = App;

    fn deref(&self) -> &Self::Target {
        self.app
    }
}

/// Converts Ratatui's frame partition into frontend-independent geometry.
pub fn frame_geometry(screen: TuiRect) -> FrameGeometry {
    let [editor_area, global_status_line, interaction_line] = TuiLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(screen);
    FrameGeometry {
        screen: from_tui_rect(screen),
        editor: from_tui_rect(editor_area),
        status: from_tui_rect(global_status_line),
        message: from_tui_rect(interaction_line),
    }
}

/// Draws one immutable normal-editor snapshot and any current overlays.
pub fn render(
    frame: &mut Frame<'_>,
    app: &App,
    snapshot: &EditorSnapshot,
    key_hints: &KeyHintState,
) {
    let overlays = app.overlay_snapshots();
    let confirmation_overlay = overlays
        .iter()
        .find(|overlay| overlay.kind == OverlayKind::Confirmation);
    // List managers keep their underlying result list open while a contextual
    // action menu is on top. The last BufferActions snapshot is therefore the
    // active menu regardless of whether it belongs to a buffer, workspace,
    // terminal, or ordinary selection.
    let action_overlay = overlays
        .iter()
        .rev()
        .find(|overlay| overlay.kind == OverlayKind::BufferActions);
    let path_overlay = if app.path_action_menu_open() {
        overlays
            .iter()
            .find(|overlay| overlay.kind == OverlayKind::PathActions)
    } else if app.path_popup_open() {
        overlays
            .iter()
            .find(|overlay| overlay.kind == OverlayKind::Path)
    } else {
        None
    };
    let app = TuiApp::new(app, &snapshot.theme);
    let editor_area = snapshot.geometry.editor;
    let global_status_line_area = to_tui_rect(snapshot.geometry.status);
    let interaction_line_area = to_tui_rect(snapshot.geometry.message);

    for pane in &snapshot.panes {
        draw_pane(frame, &app.theme, snapshot.mode, pane);
    }

    draw_status(
        frame,
        &app.theme,
        &snapshot.status,
        SessionMode::Standalone,
        global_status_line_area,
        interaction_line_area,
    );
    if app.fs_confirmation.is_some() {
        draw_fs_confirmation(frame, &app, editor_area);
    } else if app.picker.is_some() {
        draw_picker(frame, &app, editor_area);
    } else if let Some(actions) = action_overlay {
        draw_snapshot_overlay(frame, &app.theme, actions, snapshot);
    } else if app.list.is_some() {
        draw_list(frame, &app, editor_area);
    } else {
        if app.mode == Mode::Command && app.prompt_kind == PromptKind::Command {
            draw_command_palette(frame, &app, editor_area);
        } else if app.mode == Mode::Command && app.prompt_kind == PromptKind::ExternalProgram {
            draw_program_hints(frame, &app, editor_area);
            if app.program_action_menu.is_some() {
                draw_program_actions(frame, &app, editor_area);
            }
        } else if app.mode == Mode::Command
            && matches!(app.prompt_kind, PromptKind::SettingValue(_))
        {
            draw_setting_prompt(frame, &app, editor_area);
        } else if app.key_hint_mode().is_some() && key_hints.is_visible() {
            draw_key_hints(frame, &app, key_hints, editor_area);
        }
        // Language-server popups are anchored to the caret and drawn last, so
        // they sit above the text without displacing it.
        draw_hover(frame, &app, snapshot, editor_area);
        draw_signature(frame, &app, snapshot, editor_area);
        draw_completion(frame, &app, snapshot, editor_area);
        if !matches!(app.prompt_kind, PromptKind::SettingValue(_)) {
            place_prompt_cursor(frame, &snapshot.status, interaction_line_area);
        }
    }
    if let Some(confirmation) = confirmation_overlay {
        draw_snapshot_overlay(frame, &app.theme, confirmation, snapshot);
    }
    if let Some(overlay) = path_overlay {
        draw_snapshot_overlay(frame, &app.theme, overlay, snapshot);
    }
}

fn draw_setting_prompt(frame: &mut Frame<'_>, app: &TuiApp<'_>, editor_area: Rect) {
    let PromptKind::SettingValue(setting) = app.prompt_kind else {
        return;
    };
    let descriptor = setting.descriptor();
    let constraint = match descriptor.value_type {
        crate::settings::SettingType::Integer { minimum, maximum } => {
            format!("integer {minimum}–{maximum}")
        }
        crate::settings::SettingType::Text => "text".to_owned(),
        crate::settings::SettingType::Grammar
        | crate::settings::SettingType::Boolean
        | crate::settings::SettingType::Theme
        | crate::settings::SettingType::WorkspaceMode => "choice".to_owned(),
    };
    let show_error = app.status_error && !app.displayed_status_message().is_empty();
    let area = to_tui_rect(setting_popup_area(editor_area));
    if area.width < 3 || area.height < 3 {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.accent))
        .title(format!(
            " {} · {constraint} · Enter save · Esc cancel ",
            descriptor.key
        ))
        .style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.overlay_background),
        );
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(app.command.clone()), inner);
    if show_error && inner.height > 1 {
        frame.render_widget(
            Paragraph::new(app.displayed_status_message().to_owned()).style(
                Style::default()
                    .fg(app.theme.error)
                    .bg(app.theme.overlay_background),
            ),
            TuiRect::new(
                inner.x,
                inner.y.saturating_add(1),
                inner.width,
                inner.height.saturating_sub(1),
            ),
        );
    }
    let cells = app
        .command
        .chars()
        .take(app.command_cursor)
        .map(|character| character.width().unwrap_or(0))
        .sum::<usize>();
    let x = inner
        .x
        .saturating_add(cells.min(u16::MAX as usize) as u16)
        .min(inner.right().saturating_sub(1));
    frame.set_cursor_position(ScreenPosition::new(x, inner.y));
}

/// Draws a transport-owned frame without reaching back into live application
/// state. Attached clients use this path; standalone keeps its specialized
/// overlay widgets while both consume the same semantic snapshot values.
pub fn render_host_frame(frame: &mut Frame<'_>, snapshot: &HostFrame) {
    let theme = TuiTheme::new(&snapshot.editor.theme);
    for pane in &snapshot.editor.panes {
        draw_pane(frame, &theme, snapshot.editor.mode, pane);
    }
    draw_status(
        frame,
        &theme,
        &snapshot.editor.status,
        SessionMode::Persistent,
        to_tui_rect(snapshot.editor.geometry.status),
        to_tui_rect(snapshot.editor.geometry.message),
    );
    let completion_visible = snapshot
        .overlays
        .iter()
        .any(|overlay| overlay.kind == OverlayKind::Completion);
    for overlay in snapshot.overlays.iter().filter(|overlay| {
        !matches!(
            overlay.kind,
            OverlayKind::Prompt
                | OverlayKind::Completion
                | OverlayKind::Signature
                | OverlayKind::Hover
                | OverlayKind::KeyHints
        )
    }) {
        draw_snapshot_overlay(frame, &theme, overlay, &snapshot.editor);
    }
    for kind in [
        OverlayKind::Hover,
        OverlayKind::Signature,
        OverlayKind::Completion,
        OverlayKind::Prompt,
        OverlayKind::KeyHints,
    ] {
        if kind == OverlayKind::Signature && completion_visible {
            continue;
        }
        for overlay in snapshot
            .overlays
            .iter()
            .filter(|overlay| overlay.kind == kind)
        {
            draw_snapshot_overlay(frame, &theme, overlay, &snapshot.editor);
        }
    }
    place_prompt_cursor(
        frame,
        &snapshot.editor.status,
        to_tui_rect(snapshot.editor.geometry.message),
    );
}

fn draw_snapshot_overlay(
    frame: &mut Frame<'_>,
    theme: &TuiTheme,
    overlay: &OverlaySnapshot,
    editor: &EditorSnapshot,
) {
    let editor_area = editor.geometry.editor;
    if editor_area.width < 3 || editor_area.height < 3 {
        return;
    }
    let area = if overlay.layout == OverlayLayout::Setting {
        to_tui_rect(setting_popup_area(editor_area))
    } else if overlay.layout == OverlayLayout::SettingChoice {
        to_tui_rect(setting_choice_popup_area(editor_area))
    } else if overlay.layout == OverlayLayout::Preview {
        to_tui_rect(centered(editor_area, 90, 85, 28, 8))
    } else {
        match overlay.kind {
            OverlayKind::Confirmation => {
                to_tui_rect(confirmation_overlay_area(editor_area, overlay))
            }
            OverlayKind::FilePicker => to_tui_rect(centered(editor_area, 90, 85, 28, 8)),
            OverlayKind::KeyHints => {
                let height = (overlay.rows.len() as u16 + 3).clamp(3, editor_area.height.min(16));
                to_tui_rect(Rect {
                    x: editor_area.x,
                    y: editor_area.y + editor_area.height.saturating_sub(height),
                    width: editor_area.width,
                    height,
                })
            }
            OverlayKind::Completion | OverlayKind::Signature | OverlayKind::Hover => {
                let rows = overlay.rows.len().clamp(1, 12) as u16;
                anchored_snapshot(
                    editor,
                    editor_area,
                    editor_area.width.clamp(16, 80),
                    rows + 2,
                )
                .unwrap_or_else(|| to_tui_rect(centered(editor_area, 80, 40, 16, 4)))
            }
            _ => to_tui_rect(centered(editor_area, 80, 75, 28, 7)),
        }
    };
    let query_height = usize::from(!overlay.query.is_empty());
    let message_height = usize::from(overlay.message.is_some());
    let row_capacity = usize::from(area.height)
        .saturating_sub(2 + query_height + message_height)
        .max(1);
    let anchor = overlay
        .scroll_anchor
        .and_then(|anchor| anchor.checked_sub(overlay.row_offset))
        .filter(|anchor| *anchor < overlay.rows.len());
    let visible_offset = anchor
        .map(|anchor| anchor.saturating_sub(row_capacity / 2))
        .unwrap_or_default()
        .min(overlay.rows.len().saturating_sub(row_capacity));
    let visible_rows = overlay
        .rows
        .len()
        .saturating_sub(visible_offset)
        .min(row_capacity);
    let range = if overlay.total_rows > visible_rows {
        let first = overlay
            .row_offset
            .saturating_add(visible_offset)
            .saturating_add(1)
            .min(overlay.total_rows);
        let last = overlay
            .row_offset
            .saturating_add(visible_offset)
            .saturating_add(visible_rows)
            .min(overlay.total_rows);
        format!(" · {first}–{last}/{}", overlay.total_rows)
    } else {
        String::new()
    };
    let action_hints = overlay_action_hints(overlay);
    let action_hints = if action_hints.is_empty() {
        String::new()
    } else {
        format!(" · {action_hints}")
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(format!(" {}{range}{action_hints} ", overlay.title))
        .style(
            Style::default()
                .fg(theme.foreground)
                .bg(theme.overlay_background),
        );
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let mut query_height = 0;
    if !overlay.query.is_empty() {
        query_height = 1;
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("> ", Style::default().fg(theme.accent)),
                Span::raw(overlay.query.clone()),
            ])),
            TuiRect::new(inner.x, inner.y, inner.width, 1),
        );
    }
    let content = TuiRect::new(
        inner.x,
        inner.y.saturating_add(query_height),
        inner.width,
        inner.height.saturating_sub(query_height),
    );
    let show_preview =
        overlay.layout == OverlayLayout::Preview && overlay.show_preview && content.width >= 72;
    let columns = if show_preview {
        TuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
            .split(content)
    } else {
        TuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(content)
    };
    let mut lines = Vec::new();
    for (index, row) in overlay
        .rows
        .iter()
        .enumerate()
        .skip(visible_offset)
        .take(row_capacity)
    {
        let selected = overlay.selected == Some(index);
        let mut style = if selected {
            Style::default().fg(theme.foreground).bg(theme.selection)
        } else {
            Style::default().fg(theme.foreground)
        };
        // Dormant first, unavailable second: a row can be both, and being
        // unable to act is the stronger thing to say about it. Dormancy alone
        // spares the selected row, so the reader can always read what they
        // are about to act on; unavailability does not, because a row that
        // cannot answer has to say so wherever the cursor is.
        if row.dimmed && !selected {
            style = style.fg(theme.jump_text_muted);
        }
        if !row.available {
            style = style.fg(theme.muted).add_modifier(Modifier::DIM);
        }
        let detail = if row.detail.is_empty() {
            String::new()
        } else {
            format!("  {}", row.detail)
        };
        let emphasized = row
            .emphasis
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        let mut spans = row
            .label
            .chars()
            .enumerate()
            .map(|(position, character)| {
                let mut character_style = style;
                if emphasized.contains(&position) {
                    character_style = character_style.fg(theme.accent).bold();
                }
                Span::styled(character.to_string(), character_style)
            })
            .collect::<Vec<_>>();
        let mut detail_style = Style::default().fg(if row.dimmed && !selected {
            theme.jump_text_muted
        } else {
            theme.muted
        });
        if !row.available {
            detail_style = detail_style.fg(theme.muted).add_modifier(Modifier::DIM);
        }
        spans.push(Span::styled(detail, detail_style));
        lines.push(Line::from(spans));
    }
    if let Some(message) = &overlay.message {
        let style = Style::default().fg(match overlay.purpose {
            crate::snapshot::OverlayPurpose::Confirmation => theme.warning,
            crate::snapshot::OverlayPurpose::Info => theme.info,
            _ => theme.error,
        });
        lines.extend(
            message
                .split('\n')
                .map(|line| Line::styled(line.to_owned(), style)),
        );
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), columns[0]);
    if show_preview {
        let preview = match overlay.preview.as_ref() {
            Some(OverlayPreview::Text(lines)) => {
                lines.iter().cloned().map(Line::from).collect::<Vec<_>>()
            }
            Some(OverlayPreview::MatchedText { lines, emphasis }) => {
                fuzzy_matched_text_lines(&overlay.query, lines, emphasis, theme)
            }
            Some(OverlayPreview::Snippet {
                lines,
                start_row,
                focus_row,
                emphasis,
            }) => fuzzy_preview_lines(
                &overlay.query,
                lines,
                *start_row,
                *focus_row,
                emphasis,
                theme,
            ),
            Some(OverlayPreview::Binary) => vec![Line::from("Binary file")],
            Some(OverlayPreview::Unavailable(error)) => vec![Line::from(error.clone())],
            Some(OverlayPreview::Empty) | None => vec![Line::from("No preview")],
        };
        frame.render_widget(
            Paragraph::new(preview)
                .block(Block::default().borders(Borders::LEFT).title(format!(
                    " {} ",
                    overlay.preview_title.as_deref().unwrap_or("Preview")
                )))
                .style(
                    Style::default()
                        .fg(theme.muted)
                        .bg(theme.overlay_background),
                ),
            columns[1],
        );
    }
    if let Some(cursor) = overlay.query_cursor.filter(|_| query_height > 0) {
        let cells = overlay
            .query
            .chars()
            .take(cursor)
            .map(|character| character.width().unwrap_or(0))
            .sum::<usize>();
        let x = inner
            .x
            .saturating_add(2)
            .saturating_add(cells.min(u16::MAX as usize) as u16)
            .min(inner.right().saturating_sub(1));
        frame.set_cursor_position(ScreenPosition::new(x, inner.y));
    }
}

fn anchored_snapshot(
    snapshot: &EditorSnapshot,
    editor_area: Rect,
    width: u16,
    height: u16,
) -> Option<TuiRect> {
    let pane = snapshot.panes.iter().find(|pane| pane.active)?;
    if editor_area.width < 4 || editor_area.height < 4 {
        return None;
    }
    let width = width.min(editor_area.width).max(1);
    let height = height.min(editor_area.height).max(1);
    let caret_y = pane
        .body
        .y
        .saturating_add(pane.cursor_screen_row.unwrap_or(0).min(u16::MAX as usize) as u16);
    let below = caret_y.saturating_add(1);
    let bottom = editor_area.y.saturating_add(editor_area.height);
    let y = if below.saturating_add(height) <= bottom {
        below
    } else {
        caret_y.saturating_sub(height)
    }
    .clamp(editor_area.y, bottom.saturating_sub(height));
    let x = pane.body.x.clamp(
        editor_area.x,
        editor_area
            .x
            .saturating_add(editor_area.width)
            .saturating_sub(width),
    );
    Some(TuiRect::new(x, y, width, height))
}

fn draw_fs_confirmation(frame: &mut Frame<'_>, app: &TuiApp<'_>, editor_area: Rect) {
    let Some(confirmation) = &app.fs_confirmation else {
        return;
    };
    let area = to_tui_rect(centered(editor_area, 88, 80, 30, 8));
    if area.width < 3 || area.height < 3 {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.accent))
        .title(format!(
            " Filesystem plan · {}/{} · {} ",
            confirmation.selected.saturating_add(1),
            confirmation.plan.operations().len(),
            confirmation.plan.root().display()
        ))
        .style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.overlay_background),
        );
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let footer_height = u16::from(inner.height > 1);
    let [operations, footer] = TuiLayout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(footer_height)])
        .areas(inner);
    let lines = confirmation.plan.lines();
    let visible = usize::from(operations.height).max(1);
    let offset = confirmation
        .selected
        .saturating_sub(visible / 2)
        .min(lines.len().saturating_sub(visible));
    let items = lines
        .into_iter()
        .skip(offset)
        .take(visible)
        .map(ListItem::new)
        .collect::<Vec<_>>();
    let mut state =
        ListState::default().with_selected(Some(confirmation.selected.saturating_sub(offset)));
    StatefulWidget::render(
        List::new(items)
            .style(
                Style::default()
                    .fg(app.theme.foreground)
                    .bg(app.theme.overlay_background),
            )
            .highlight_style(Style::default().bg(app.theme.selection)),
        operations,
        frame.buffer_mut(),
        &mut state,
    );
    if footer_height > 0 {
        frame.render_widget(
            Paragraph::new(
                "↑/↓ review · Enter apply (trash deletes) · P permanently delete · Esc cancel",
            )
            .style(Style::default().fg(app.theme.muted)),
            footer,
        );
    }
}

fn draw_pane(frame: &mut Frame<'_>, theme: &TuiTheme, mode: Mode, pane: &PaneSnapshot) {
    if !pane.drawable {
        return;
    }
    let area = to_tui_rect(pane.area);
    let body = to_tui_rect(pane.body);
    let background = theme.pane_background(pane.active);
    let title = pane_title_text(
        &pane.title.name,
        pane.title.dirty,
        pane.title.read_only,
        pane.title.maximized,
        usize::from(area.width),
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(if pane.active {
            theme.accent
        } else {
            theme.muted
        }))
        .title(title)
        .style(Style::default().fg(theme.foreground).bg(background));
    frame.render_widget(block, area);

    if let Some(terminal) = pane.terminal.as_ref() {
        draw_terminal(frame, theme, mode, pane.active, pane.dimmed, terminal, body);
        return;
    }

    let lines = pane
        .rows
        .iter()
        .map(|row| snapshot_line(theme, mode, pane, row))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.foreground).bg(background)),
        body,
    );
}

/// Draws a terminal pane's cells.
///
/// Every cell carries its own colour, so nothing here consults a scope or a
/// syntax highlight. The theme resolves what
/// [`TerminalColor::Default`](crate::terminal::Color::Default) means, and it
/// means the editor's own foreground and background — which is what keeps a
/// shell readable in a light theme instead of assuming a black screen. The
/// active caret is editor chrome rather than child content, so it follows the
/// current mode's cursor colour just as it does over a file.
fn draw_terminal(
    frame: &mut Frame<'_>,
    theme: &TuiTheme,
    mode: Mode,
    active: bool,
    dimmed: bool,
    terminal: &TerminalView,
    body: TuiRect,
) {
    let background = theme.pane_background(active);
    let lines = terminal
        .rows
        .iter()
        .enumerate()
        .map(|(row, cells)| terminal_line(theme, mode, active, dimmed, terminal, row, cells))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.foreground).bg(background)),
        body,
    );
}

fn terminal_line(
    theme: &TuiTheme,
    mode: Mode,
    active: bool,
    dimmed: bool,
    terminal: &TerminalView,
    row: usize,
    cells: &[TerminalCell],
) -> Line<'static> {
    let background = theme.pane_background(active);
    let cursor = terminal
        .cursor
        .filter(|(cursor_row, _)| *cursor_row == row)
        .map(|(_, column)| column);
    let mut spans = Vec::new();
    for (column, cell) in cells.iter().enumerate() {
        // A double-width character already painted its second cell; drawing
        // the spacer would push the rest of the row one column right.
        if cell.width == 0 {
            continue;
        }
        let mut style = terminal_style(theme, active, cell);
        if dimmed {
            style = style.fg(theme.jump_text_muted);
        }
        if let Some(highlight) = terminal.highlights.iter().rev().find(|highlight| {
            highlight.row == row
                && column >= highlight.start_column
                && column < highlight.end_column
        }) {
            use crate::terminal::TerminalHighlightKind as Kind;
            style = match highlight.kind {
                Kind::Match => style.bg(theme.fuzzy_match_secondary),
                Kind::ActiveMatch => style.bg(theme.fuzzy_match_primary),
                Kind::Selection => style.bg(theme.selection_primary),
                Kind::JumpLabelImmediate => Style::default()
                    .fg(theme.jump_label_immediate)
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
                Kind::JumpLabelPrefix => Style::default()
                    .fg(theme.jump_label_primary)
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
                Kind::JumpLabelSuffix => Style::default()
                    .fg(theme.jump_label_secondary)
                    .bg(background),
            };
        }
        if cursor == Some(column) {
            style = if active {
                // Like an editor caret, the active terminal caret follows the
                // current NOR/INS/SEL colour. Rebuild the style so a child's
                // reverse or hidden attributes cannot mask that mode colour.
                Style::default().fg(theme.background).bg(theme.cursor(mode))
            } else {
                style.add_modifier(Modifier::UNDERLINED)
            };
        }
        spans.push(Span::styled(cell.text(), style));
    }
    Line::from(spans)
}

fn terminal_style(theme: &TuiTheme, active: bool, cell: &TerminalCell) -> Style {
    use crate::terminal::Attributes;

    let mut style = Style::default()
        .fg(terminal_color(theme, cell.foreground, theme.foreground))
        .bg(terminal_color(
            theme,
            cell.background,
            theme.pane_background(active),
        ));
    for (attribute, modifier) in [
        (Attributes::BOLD, Modifier::BOLD),
        (Attributes::DIM, Modifier::DIM),
        (Attributes::ITALIC, Modifier::ITALIC),
        (Attributes::UNDERLINE, Modifier::UNDERLINED),
        (Attributes::BLINK, Modifier::SLOW_BLINK),
        (Attributes::REVERSE, Modifier::REVERSED),
        (Attributes::HIDDEN, Modifier::HIDDEN),
        (Attributes::STRIKETHROUGH, Modifier::CROSSED_OUT),
    ] {
        if cell.attributes.contains(attribute) {
            style = style.add_modifier(modifier);
        }
    }
    style
}

fn terminal_color(
    _theme: &TuiTheme,
    color: crate::terminal::Color,
    fallback: ratatui::style::Color,
) -> ratatui::style::Color {
    match color {
        crate::terminal::Color::Default => fallback,
        crate::terminal::Color::Indexed(index) => ratatui::style::Color::Indexed(index),
        crate::terminal::Color::Rgb(red, green, blue) => {
            ratatui::style::Color::Rgb(red, green, blue)
        }
    }
}

fn snapshot_line(
    theme: &TuiTheme,
    mode: Mode,
    pane: &PaneSnapshot,
    row: &SnapshotRow,
) -> Line<'static> {
    let background = theme.pane_background(pane.active);
    let row = match row {
        SnapshotRow::Placeholder => {
            return Line::from(Span::styled("~", Style::default().fg(theme.muted)));
        }
        // Filler stands for lines this side does not have, so it is drawn as
        // an absence: a hatch across the text column with the gutter left
        // blank, which reads as "nothing here" rather than as an empty line
        // of the file.
        SnapshotRow::Filler => {
            return Line::from(vec![
                Span::raw(" ".repeat(pane.gutter_width)),
                Span::styled(
                    "╱".repeat(pane.text_width),
                    Style::default().fg(theme.muted),
                ),
            ]);
        }
        // Space held open around centred content. It stands for nothing, so
        // it is drawn as nothing rather than as a past-the-end marker.
        SnapshotRow::Padding => return Line::default(),
        SnapshotRow::Text(row) => row,
    };
    let mut spans = Vec::new();
    if pane.signs {
        spans.push(Span::styled(
            if row.continuation {
                " ".to_owned()
            } else {
                row.diagnostic_sign
                    .map_or_else(|| " ".to_owned(), |severity| severity.sign().to_string())
            },
            Style::default().fg(severity_color(theme, row.diagnostic_sign)),
        ));
    }
    if pane.line_numbers {
        let split_fold_change = pane.gutter_width > pane.line_digits + 3 + usize::from(pane.signs);
        let line_style = Style::default().fg(if row.cursor_row && pane.active {
            theme.accent
        } else {
            theme.muted
        });
        if row.continuation {
            spans.push(Span::styled(" ".repeat(pane.line_digits), line_style));
            spans.push(Span::styled("↪", Style::default().fg(theme.muted)));
            if split_fold_change {
                spans.push(Span::raw(" "));
            }
        } else {
            spans.push(Span::styled(
                format!(
                    "{:>digits$}",
                    row.document_row + 1,
                    digits = pane.line_digits
                ),
                line_style,
            ));
            let fold_style = || {
                let style = Style::default().fg(theme.accent);
                if row.cursor_row && pane.active {
                    style.add_modifier(Modifier::BOLD)
                } else {
                    style
                }
            };
            let changed = pane.changes.then(|| change_marker(theme, row)).flatten();
            if split_fold_change {
                spans.push(Span::styled(
                    if row.folded { "▸" } else { " " },
                    if row.folded {
                        fold_style()
                    } else {
                        Style::default().fg(theme.muted)
                    },
                ));
                let (marker, color) = changed.unwrap_or((" ", theme.muted));
                spans.push(Span::styled(marker, Style::default().fg(color)));
            } else {
                let (marker, marker_style) = if let Some((marker, color)) = changed {
                    (marker, Style::default().fg(color))
                } else if row.folded {
                    ("▸", fold_style())
                } else {
                    (" ", Style::default().fg(theme.muted))
                };
                spans.push(Span::styled(marker, marker_style));
            }
        }
        spans.push(Span::styled("│", Style::default().fg(theme.muted)));
        spans.push(Span::raw(" "));
    }
    if pane.changes && !pane.line_numbers {
        let (marker, color) = change_marker(theme, row).unwrap_or((" ", theme.muted));
        spans.push(Span::styled(marker, Style::default().fg(color)));
    }
    // The margin a centred page asks for. It sits after the gutter so line
    // numbers and marks stay against the edge of the pane, and it carries no
    // style of its own: it is space, not text.
    if pane.content_indent > 0 {
        spans.push(Span::raw(" ".repeat(pane.content_indent)));
    }
    let dim_muted = if pane.dimmed {
        theme.jump_text_muted
    } else {
        theme.muted
    };
    spans.extend(row.runs.iter().map(|run| {
        let style = match run.kind {
            TextRunKind::Text {
                role,
                scope,
                diagnostic,
                directory,
                count,
            } => {
                let style = text_run_style(
                    theme,
                    mode,
                    role,
                    scope,
                    diagnostic,
                    directory,
                    count,
                    row.diff,
                    row.compared,
                );
                let style = if !pane.active && style.bg == Some(theme.background) {
                    style.bg(background)
                } else {
                    style
                };
                let style = if role == TextRole::Plain {
                    match row.notification_severity {
                        Some(NotificationSeverity::Error) => style.fg(theme.error),
                        Some(NotificationSeverity::Warning) => style.fg(theme.warning),
                        Some(NotificationSeverity::Info) => style.fg(theme.info),
                        None => style,
                    }
                } else {
                    style
                };
                // The caret is editor chrome rather than document text: while
                // a command prompt dims the panes it keeps the colour that
                // names the mode, so CMD stays identifiable in every pane.
                // `goto-word` still dims it with everything else, because
                // there the caret is one more thing the labels have to stand
                // out from.
                let caret = matches!(
                    role,
                    TextRole::Caret | TextRole::PrimaryCaret | TextRole::ReplaceCaret
                );
                if pane.dimmed && (pane.jump_active || !caret) {
                    style.fg(dim_muted)
                } else {
                    style
                }
            }
            TextRunKind::JumpLabel(part) => {
                use crate::jump_labels::LabelPart;
                let style = Style::default()
                    .fg(match part {
                        LabelPart::Immediate => theme.jump_label_immediate,
                        LabelPart::Prefix => theme.jump_label_primary,
                        LabelPart::Suffix => theme.jump_label_secondary,
                    })
                    .bg(background);
                match part {
                    LabelPart::Immediate | LabelPart::Prefix => style.add_modifier(Modifier::BOLD),
                    LabelPart::Suffix => style,
                }
            }
            TextRunKind::InlineDiagnostic(severity) => Style::default()
                .fg(if pane.dimmed {
                    dim_muted
                } else {
                    severity_color(theme, Some(severity))
                })
                .bg(background)
                .add_modifier(Modifier::ITALIC),
            TextRunKind::FoldMarker => Style::default()
                .fg(dim_muted)
                .bg(background)
                .add_modifier(Modifier::ITALIC),
            // Every read-only annotation is drawn the same way, whatever the
            // buffer: muted so it reads as commentary, italic like the other
            // text that is not in the document.
            TextRunKind::Hint => Style::default()
                .fg(dim_muted)
                .bg(background)
                .add_modifier(Modifier::ITALIC),
        };
        Span::styled(run.text.clone(), style)
    }));
    Line::from(spans)
}

/// The one-cell symbol and colour for a changed logical line.
///
/// A live side-by-side comparison owns the column while it is open; otherwise
/// the symbol describes the line against Git's staged text. Removed lines have
/// no row of their own, so both edge positions use `-` on the surviving row
/// that closes over the gap.
fn change_marker(
    theme: &TuiTheme,
    row: &crate::snapshot::VisibleRow,
) -> Option<(&'static str, ratatui::style::Color)> {
    match (row.compared, row.change) {
        (Some(Change::Added), _) => Some(("+", theme.change_added)),
        (Some(Change::Changed), _) => Some(("~", theme.change_modified)),
        (Some(Change::Removed), _) => Some(("-", theme.change_removed)),
        (None, Some(LineChange::Added)) => Some(("+", theme.change_added)),
        (None, Some(LineChange::Modified)) => Some(("~", theme.change_modified)),
        (None, Some(LineChange::RemovedAbove | LineChange::RemovedBelow)) => {
            Some(("-", theme.change_removed))
        }
        (None, None) => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn text_run_style(
    theme: &TuiTheme,
    mode: Mode,
    role: TextRole,
    scope: Option<crate::syntax::Scope>,
    diagnostic: Option<crate::lsp::Severity>,
    directory: bool,
    count: Option<CountKind>,
    diff: Option<DiffLine>,
    compared: Option<Change>,
) -> Style {
    // A diff line's colour comes from what the line is, not from a syntax
    // scope, and it reuses the gutter's palette so that added text is the same
    // green wherever the reader meets it. The changed-file list's two counts
    // are the same claim in a narrower place, so they are painted from the
    // same two theme colours rather than from any of their own.
    let foreground = match count {
        Some(CountKind::Added) => theme.change_added,
        Some(CountKind::Removed) => theme.change_removed,
        None => match diff {
            Some(DiffLine::Added) => theme.change_added,
            Some(DiffLine::Removed) => theme.change_removed,
            Some(DiffLine::Hunk) => theme.accent,
            Some(DiffLine::Meta) => theme.muted,
            None if directory => theme.directory,
            None => scope
                .and_then(|scope| theme.syntax_color(scope))
                .unwrap_or(theme.foreground),
        },
    };
    // The fill says which side of a comparison a line belongs to, and it sits
    // underneath everything: a selection or a caret still paints over it, so
    // where the person is stays as visible in a diff as anywhere else. A theme
    // that names no fill keeps the ordinary background and lets the gutter bar
    // carry the difference on its own.
    let background = compared
        .and_then(|change| match change {
            Change::Added => theme.diff_added,
            Change::Removed => theme.diff_removed,
            Change::Changed => theme.diff_changed,
        })
        .unwrap_or(theme.background);
    let normal = Style::default().fg(foreground).bg(background);
    let base = match role {
        TextRole::Plain => normal,
        TextRole::Selected => normal.bg(theme.selection),
        TextRole::PrimarySelected => normal.bg(theme.selection_primary),
        TextRole::PrimaryCaret => Style::default()
            .fg(theme.background)
            .bg(theme.cursor_select),
        TextRole::ReplaceCaret => Style::default()
            .fg(theme.background)
            .bg(theme.cursor_insert),
        TextRole::Caret => Style::default().fg(theme.background).bg(theme.cursor(mode)),
    };
    match diagnostic {
        Some(_)
            if !matches!(
                role,
                TextRole::PrimaryCaret | TextRole::ReplaceCaret | TextRole::Caret
            ) =>
        {
            base.add_modifier(Modifier::UNDERLINED)
        }
        _ => base,
    }
}

fn severity_color(
    theme: &TuiTheme,
    severity: Option<crate::lsp::Severity>,
) -> ratatui::style::Color {
    use crate::lsp::Severity;
    match severity {
        Some(Severity::Error) => theme.error,
        Some(Severity::Warning) => theme.accent,
        Some(Severity::Information | Severity::Hint) | None => theme.muted,
    }
}

/// Which workspace mode the person is using, as the status row reports it.
///
/// This is a property of the frontend rather than of the frame it draws. A
/// persistent host renders frames it never displays itself, so the same
/// snapshot is standalone or persistent according to who drew it, and each of
/// the two render paths already knows which it is: [`render`] holds live
/// application state, and [`render_host_frame`] holds a frame that arrived
/// over the transport. Carrying it in the snapshot instead would put a
/// question the host cannot answer about itself onto the wire.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionMode {
    /// The TUI and the workspace host are this one process.
    Standalone,
    /// This TUI is attached to a workspace host running elsewhere.
    Persistent,
}

impl SessionMode {
    fn label(self) -> &'static str {
        match self {
            Self::Standalone => "standalone",
            Self::Persistent => "persistent",
        }
    }
}

fn draw_status(
    frame: &mut Frame<'_>,
    theme: &TuiTheme,
    status: &StatusSnapshot,
    session: SessionMode,
    global_status_line_area: TuiRect,
    interaction_line_area: TuiRect,
) {
    if let Some(action) = status.long_running_action.as_ref() {
        draw_long_running_action(
            frame,
            theme,
            action,
            status.notification_counts,
            global_status_line_area,
        );
    } else {
        draw_normal_status(frame, theme, status, session, global_status_line_area);
    }
    // An active prompt owns the interaction line's raw text so its cursor
    // column, computed against the untruncated string, stays correct; only
    // an action echo is clipped to what this frame actually has room for.
    let interaction_line = if status.prompt_cursor_column.is_none() {
        clip_interaction_line(
            &status.interaction_line,
            usize::from(interaction_line_area.width),
        )
    } else {
        status.interaction_line.clone()
    };
    frame.render_widget(
        Paragraph::new(interaction_line).style(
            Style::default()
                .fg(if status.interaction_line_error {
                    theme.error
                } else {
                    theme.foreground
                })
                .bg(theme.background),
        ),
        interaction_line_area,
    );
}

/// Truncates the interaction line's action echo to the display cells this
/// frame's row actually has, cutting to the first line first if the text
/// has more than one. Appends a trailing `...` — three literal dots, not
/// the `…` glyph `clip_with_ellipsis` uses for compact status-row labels —
/// whenever either cut removed anything; `:not` is where the untruncated
/// text is read in full regardless.
///
/// An echo always has the `spelling (detail)` shape `report_completed_action`
/// composes, so a closing `)` on the (possibly multiline) original is kept
/// after the marker rather than silently dropped with whatever followed it.
fn clip_interaction_line(text: &str, width: usize) -> String {
    let mut lines = text.lines();
    let first_line = lines.next().unwrap_or("");
    let has_more_lines = lines.next().is_some();
    if !has_more_lines && UnicodeWidthStr::width(first_line) <= width {
        return first_line.to_owned();
    }
    let closing_paren = text.ends_with(')') && !first_line.ends_with(')');
    const MARKER: &str = "...";
    let marker_width = UnicodeWidthStr::width(MARKER) + usize::from(closing_paren);
    if width <= marker_width {
        // Not enough room to show the marker meaningfully: keep as much
        // raw text as fits rather than a truncated or misleading marker.
        let mut clipped = String::new();
        let mut used = 0;
        for grapheme in first_line.graphemes(true) {
            let cells = UnicodeWidthStr::width(grapheme);
            if used + cells > width {
                break;
            }
            clipped.push_str(grapheme);
            used += cells;
        }
        return clipped;
    }
    let budget = width - marker_width;
    let mut clipped = String::new();
    let mut used = 0;
    for grapheme in first_line.graphemes(true) {
        let cells = UnicodeWidthStr::width(grapheme);
        if used + cells > budget {
            break;
        }
        clipped.push_str(grapheme);
        used += cells;
    }
    clipped.push_str(MARKER);
    if closing_paren {
        clipped.push(')');
    }
    clipped
}

fn draw_normal_status(
    frame: &mut Frame<'_>,
    theme: &TuiTheme,
    status: &StatusSnapshot,
    session: SessionMode,
    status_area: TuiRect,
) {
    let mode_label = format!(" {} ", status.mode.label());
    // An unnumbered workspace contributes nothing rather than a placeholder:
    // the marker is a key somebody can press, so showing one where no key
    // works would be worse than showing none.
    let number_label = status
        .workspace_number
        .map(|number| format!("[S{number}] "))
        .unwrap_or_default();
    let left_prefix = format!("│ {} │ {number_label}Workspace: ", session.label());
    let left_suffix = format!(
        "{}{} ",
        if status.dirty { " [+]" } else { "" },
        if status.read_only { " [RO]" } else { "" }
    );
    let count = status.selection_count;
    // The progress percentage joins the cursor with a middle dot rather than a
    // bar: it is another reading of the same position, not a separate field.
    let right = format!(
        " {}:{} · {}%{}{} ",
        status.cursor.row + 1,
        status.cursor.col + 1,
        status.progress_percent(),
        if count > 1 {
            format!(" │ {count} sel")
        } else {
            String::new()
        },
        [status.git_summary.as_ref(), status.lsp_summary.as_ref()]
            .into_iter()
            .flatten()
            .map(|summary| format!(" │ {summary}"))
            .collect::<String>(),
    );
    let status_width = status_area.width as usize;
    let fixed_width = UnicodeWidthStr::width(mode_label.as_str())
        + UnicodeWidthStr::width(left_prefix.as_str())
        + UnicodeWidthStr::width(left_suffix.as_str())
        + UnicodeWidthStr::width(right.as_str());
    let base = theme.status_style();
    let neutral = base.fg.unwrap_or(theme.foreground);
    let full_indicator = notification_indicator(theme, status.notification_counts, false, neutral);
    let full_width = indicator_width(&full_indicator);
    let shortest_path_width = if status.workspace_directory.is_empty() {
        0
    } else {
        3
    };
    let indicator = if fixed_width
        .saturating_add(shortest_path_width)
        .saturating_add(full_width)
        <= status_width
    {
        full_indicator
    } else {
        notification_indicator(theme, status.notification_counts, true, neutral)
    };
    let indicator_width = indicator_width(&indicator);
    let content_width = status_width.saturating_sub(indicator_width);
    let path_width = content_width.saturating_sub(fixed_width);
    let directory = clip_path_start(&status.workspace_directory, path_width);
    let left = format!("{left_prefix}{directory}{left_suffix}");
    let base_width = UnicodeWidthStr::width(mode_label.as_str())
        + UnicodeWidthStr::width(left.as_str())
        + UnicodeWidthStr::width(right.as_str());
    let gap = content_width.saturating_sub(base_width);
    frame.render_widget(Paragraph::new("").style(base), status_area);
    let spans = vec![
        Span::styled(mode_label, theme.mode_status_style(status.mode)),
        Span::styled(left, base),
        Span::styled(" ".repeat(gap), base),
        Span::styled(right, base),
    ];
    let content_area = TuiRect::new(
        status_area.x,
        status_area.y,
        u16::try_from(content_width).unwrap_or(status_area.width),
        status_area.height,
    );
    frame.render_widget(Paragraph::new(Line::from(spans)).style(base), content_area);
    draw_notification_indicator(frame, base, indicator, status_area);
}

/// Formats a pane's top-border title, trimming a long path from its start
/// rather than letting ratatui hard-clip the end of the whole title and
/// swallow the filename that identifies the buffer.
///
/// The buffer's own markers come first and the maximized view last: the first
/// two say what the buffer is, while the third says how this pane is being
/// presented, and only while it is.
fn pane_title_text(
    name: &str,
    dirty: bool,
    read_only: bool,
    maximized: Option<MaximizedView>,
    pane_width: usize,
) -> String {
    let dirty_marker = if dirty { " [+]" } else { "" };
    let read_only_marker = if read_only { " [RO]" } else { "" };
    let maximized_marker = match maximized {
        Some(MaximizedView::Zen) => " [zen]",
        Some(MaximizedView::Fullscreen) => " [fullscreen]",
        None => "",
    };
    let fixed_width = 2 // border cells
        + 2 // leading/trailing padding spaces
        + UnicodeWidthStr::width(dirty_marker)
        + UnicodeWidthStr::width(read_only_marker)
        + UnicodeWidthStr::width(maximized_marker);
    let name_width = pane_width.saturating_sub(fixed_width);
    let name = clip_path_start(name, name_width);
    format!(" {name}{dirty_marker}{read_only_marker}{maximized_marker} ")
}

/// Keeps the identifying end of a workspace path when the status row is
/// crowded. Grapheme and display-cell accounting prevents a wide or combining
/// character from being split even though the snapshot itself is UTF-8.
fn clip_path_start(path: &str, width: usize) -> String {
    if UnicodeWidthStr::width(path) <= width {
        return path.to_owned();
    }
    const PREFIX: &str = "...";
    let prefix_width = UnicodeWidthStr::width(PREFIX);
    if width < prefix_width {
        return String::new();
    }
    if width == prefix_width {
        return PREFIX.to_owned();
    }

    let budget = width - prefix_width;
    let mut tail = Vec::new();
    let mut used = 0;
    for grapheme in path.graphemes(true).rev() {
        let cells = UnicodeWidthStr::width(grapheme);
        if used + cells > budget {
            break;
        }
        tail.push(grapheme);
        used += cells;
    }
    tail.reverse();
    format!("{PREFIX}{}", tail.concat())
}

fn notification_indicator(
    theme: &TuiTheme,
    counts: NotificationCounts,
    compact: bool,
    neutral: ratatui::style::Color,
) -> Vec<(String, ratatui::style::Color)> {
    if counts.total() == 0 {
        return Vec::new();
    }
    let color = |severity| match severity {
        NotificationSeverity::Error => theme.error,
        NotificationSeverity::Warning => theme.warning,
        NotificationSeverity::Info => theme.info,
    };
    let mut result = vec![(" │ ".to_owned(), neutral)];
    if compact {
        let severity = counts.highest().expect("non-empty counts have a severity");
        result.push((
            format!("{}{}", severity.indicator(), counts.total()),
            color(severity),
        ));
        result.push((" ".to_owned(), neutral));
        return result;
    }
    for (severity, count) in [
        (NotificationSeverity::Error, counts.errors),
        (NotificationSeverity::Warning, counts.warnings),
        (NotificationSeverity::Info, counts.infos),
    ] {
        if count > 0 {
            result.push((format!("{}{count}", severity.indicator()), color(severity)));
            result.push((" ".to_owned(), neutral));
        }
    }
    result
}

fn indicator_width(indicator: &[(String, ratatui::style::Color)]) -> usize {
    indicator
        .iter()
        .map(|(text, _)| UnicodeWidthStr::width(text.as_str()))
        .sum()
}

fn draw_notification_indicator(
    frame: &mut Frame<'_>,
    base: Style,
    indicator: Vec<(String, ratatui::style::Color)>,
    area: TuiRect,
) {
    let width = indicator_width(&indicator).min(usize::from(area.width));
    if width == 0 {
        return;
    }
    let indicator_area = TuiRect::new(
        area.right()
            .saturating_sub(u16::try_from(width).unwrap_or(area.width)),
        area.y,
        u16::try_from(width).unwrap_or(area.width),
        area.height,
    );
    let spans = indicator
        .into_iter()
        .map(|(text, color)| Span::styled(text, base.fg(color)))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(base),
        indicator_area,
    );
}

fn draw_long_running_action(
    frame: &mut Frame<'_>,
    theme: &TuiTheme,
    action: &crate::snapshot::LongRunningActionSnapshot,
    notification_counts: NotificationCounts,
    area: TuiRect,
) {
    let base = theme.status_style();
    frame.render_widget(Paragraph::new("").style(base), area);
    let indicator = notification_indicator(
        theme,
        notification_counts,
        true,
        base.fg.unwrap_or(theme.foreground),
    );
    let indicator_width = indicator_width(&indicator).min(usize::from(area.width));
    let width = usize::from(area.width).saturating_sub(indicator_width);
    if width == 0 {
        draw_notification_indicator(frame, base, indicator, area);
        return;
    }
    let action_area = TuiRect::new(
        area.x,
        area.y,
        u16::try_from(width).unwrap_or(area.width),
        area.height,
    );
    let elapsed = action.elapsed_millis / 1_000;
    let cancel = action
        .cancel_hint
        .as_deref()
        .map_or_else(String::new, |hint| format!(" · {hint}"));
    let label = format!(
        " {} · {} · {elapsed}s{cancel} ",
        action.label, action.detail
    );
    let label = clip_with_ellipsis(&label, width.saturating_sub(2));
    let label_width = UnicodeWidthStr::width(label.as_str());
    let gap_width = width.saturating_sub(label_width).saturating_sub(1);
    let spans = vec![
        Span::styled(label, base),
        Span::styled(" ".repeat(gap_width), base),
        Span::styled(
            long_running_spinner_frame(action.elapsed_millis),
            base.fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)).style(base), action_area);
    draw_notification_indicator(frame, base, indicator, area);
}

fn long_running_spinner_frame(elapsed_millis: u64) -> &'static str {
    const FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
    FRAMES[((elapsed_millis / 80) % FRAMES.len() as u64) as usize]
}

fn clip_with_ellipsis(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let budget = width.saturating_sub(1);
    let mut clipped = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let cells = UnicodeWidthStr::width(grapheme);
        if used + cells > budget {
            break;
        }
        clipped.push_str(grapheme);
        used += cells;
    }
    clipped.push('…');
    clipped
}

fn place_prompt_cursor(
    frame: &mut Frame<'_>,
    status: &StatusSnapshot,
    interaction_line_area: TuiRect,
) {
    let Some(column) = status.prompt_cursor_column else {
        return;
    };
    let x = interaction_line_area
        .x
        .saturating_add(column.min(u16::MAX as usize) as u16)
        .min(interaction_line_area.right().saturating_sub(1));
    frame.set_cursor_position(ScreenPosition::new(x, interaction_line_area.y));
}

fn draw_picker(frame: &mut Frame<'_>, app: &TuiApp<'_>, editor_area: Rect) {
    let Some(picker) = &app.picker else {
        return;
    };
    if let Some(finder) = &app.finder
        && finder.mode == crate::finder::FinderMode::Resources
    {
        draw_resource_finder(frame, app, editor_area, picker, finder);
        return;
    }
    let area = to_tui_rect(centered(editor_area, 90, 85, 12, 7));
    if area.width < 4 || area.height < 4 {
        return;
    }
    let progress = if picker.loading { "scanning" } else { "ready" };
    let skipped = if picker.skipped > 0 {
        format!(" · {} skipped", picker.skipped)
    } else {
        String::new()
    };
    let title = if app.finder.is_some() {
        format!(
            " Find · Files · {} · {progress} · {}/{}{}{} · Tab buffers + terminals · Ctrl-t preview ",
            picker.root.display(),
            picker.matches.len(),
            picker.entries.len(),
            skipped,
            if picker.limited {
                " · result limit reached"
            } else {
                ""
            }
        )
    } else {
        format!(
            " {} · {} · {progress} · {}/{}{}{} · Ctrl-t preview ",
            picker.kind.title(),
            picker.root.display(),
            picker.matches.len(),
            picker.entries.len(),
            skipped,
            if picker.limited {
                " · result limit reached"
            } else {
                ""
            }
        )
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.accent))
        .title(title)
        .style(Style::default().bg(app.theme.overlay_background));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let rows = TuiLayout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    let query = if picker.query.is_empty() {
        Line::from(Span::styled(
            if picker.kind == crate::file_picker::FilePickerKind::Contents {
                "> type to fuzzy-search contents"
            } else {
                "> type to fuzzy-find"
            },
            Style::default().fg(app.theme.muted),
        ))
    } else {
        Line::from(vec![
            Span::styled("> ", Style::default().fg(app.theme.accent)),
            Span::styled(
                picker.query.clone(),
                Style::default().fg(app.theme.foreground),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(query), rows[0]);
    let query_cells = picker
        .query
        .chars()
        .take(picker.query_cursor)
        .map(|character| character.width().unwrap_or(0))
        .sum::<usize>();
    let query_x = rows[0]
        .x
        .saturating_add(2)
        .saturating_add(query_cells.min(u16::MAX as usize) as u16)
        .min(rows[0].right().saturating_sub(1));
    frame.set_cursor_position(ScreenPosition::new(query_x, rows[0].y));

    let show_preview = picker.show_preview && rows[1].width >= 72;
    let columns = if show_preview {
        TuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
            .split(rows[1])
    } else {
        TuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(rows[1])
    };
    let path_width = columns[0].width.saturating_sub(3) as usize;
    let items = if let Some(error) = &picker.error {
        vec![
            ListItem::new(format!("Scan failed: {error}"))
                .style(Style::default().fg(app.theme.error)),
        ]
    } else if picker.matches.is_empty() {
        let message = if picker.loading {
            "Scanning…"
        } else if picker.entries.is_empty() {
            if picker.kind == crate::file_picker::FilePickerKind::Contents {
                "No searchable text below this root"
            } else {
                "No files below this root"
            }
        } else if picker.kind == crate::file_picker::FilePickerKind::Contents {
            "No matching content"
        } else {
            "No matching files"
        };
        vec![ListItem::new(message).style(Style::default().fg(app.theme.muted))]
    } else {
        let visible_rows = columns[0].height.max(1) as usize;
        let window_start = picker
            .selected
            .saturating_add(1)
            .saturating_sub(visible_rows);
        picker
            .matches
            .iter()
            .skip(window_start)
            .take(visible_rows)
            .filter_map(|found| picker.view(found.entry).map(|entry| (entry, found)))
            .map(|(entry, found)| {
                ListItem::new(matched_path_line(
                    &entry.label(),
                    &entry.match_positions_in_label(&found.positions),
                    path_width,
                    app.theme.foreground,
                    app.theme.accent,
                ))
            })
            .collect::<Vec<_>>()
    };
    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.selection)
                .bold(),
        )
        .highlight_symbol("▸ ");
    let visible_rows = columns[0].height.max(1) as usize;
    let window_start = picker
        .selected
        .saturating_add(1)
        .saturating_sub(visible_rows);
    let selected = (!picker.matches.is_empty()).then_some(picker.selected - window_start);
    let mut state = ListState::default().with_selected(selected);
    StatefulWidget::render(list, columns[0], frame.buffer_mut(), &mut state);

    if show_preview {
        let preview_block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(app.theme.muted))
            .title(" Preview ");
        let lines = match picker.preview.as_ref() {
            Some(crate::file_picker::FilePreview::Text(lines)) => lines
                .iter()
                .map(|line| Line::from(line.clone()))
                .collect::<Vec<_>>(),
            Some(crate::file_picker::FilePreview::Snippet(snippet)) => fuzzy_preview_lines(
                &picker.query,
                &snippet.lines,
                snippet.start_row,
                snippet.focus_row,
                &snippet.emphasis,
                &app.theme,
            ),
            Some(crate::file_picker::FilePreview::Binary) => {
                vec![Line::from("<Binary file>")]
            }
            Some(crate::file_picker::FilePreview::Directory(lines)) => lines
                .iter()
                .map(|line| Line::from(line.clone()))
                .collect::<Vec<_>>(),
            Some(crate::file_picker::FilePreview::Unreadable(error)) => {
                vec![Line::from(format!("<Preview unavailable: {error}>"))]
            }
            None => vec![Line::from("<No selected file>")],
        };
        frame.render_widget(
            Paragraph::new(lines)
                .block(preview_block)
                .style(Style::default().fg(app.theme.foreground)),
            columns[1],
        );
    }
}

fn draw_resource_finder(
    frame: &mut Frame<'_>,
    app: &TuiApp<'_>,
    editor_area: Rect,
    picker: &crate::file_picker::FilePicker,
    finder: &crate::finder::ResourceFinder,
) {
    let area = to_tui_rect(centered(editor_area, 90, 85, 12, 7));
    if area.width < 4 || area.height < 4 {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.accent))
        .title(format!(
            " Find · {} · {}/{} · Tab files · Ctrl-t preview ",
            finder.mode.title(),
            finder.matches.len(),
            finder.items.len()
        ))
        .style(Style::default().bg(app.theme.overlay_background));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let rows = TuiLayout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    let query = if picker.query.is_empty() {
        Line::from(Span::styled(
            "> type to find buffers and terminals",
            Style::default().fg(app.theme.muted),
        ))
    } else {
        Line::from(vec![
            Span::styled("> ", Style::default().fg(app.theme.accent)),
            Span::styled(
                picker.query.clone(),
                Style::default().fg(app.theme.foreground),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(query), rows[0]);
    let query_cells = picker
        .query
        .chars()
        .take(picker.query_cursor)
        .map(|character| character.width().unwrap_or(0))
        .sum::<usize>();
    let query_x = rows[0]
        .x
        .saturating_add(2)
        .saturating_add(query_cells.min(u16::MAX as usize) as u16)
        .min(rows[0].right().saturating_sub(1));
    frame.set_cursor_position(ScreenPosition::new(query_x, rows[0].y));

    let show_preview = picker.show_preview && rows[1].width >= 72;
    let columns = if show_preview {
        TuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
            .split(rows[1])
    } else {
        TuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(rows[1])
    };
    let visible_rows = columns[0].height.max(1) as usize;
    let window_start = finder
        .selected
        .saturating_add(1)
        .saturating_sub(visible_rows);
    let items = if finder.matches.is_empty() {
        vec![
            ListItem::new("No matching buffers or terminals")
                .style(Style::default().fg(app.theme.muted)),
        ]
    } else {
        finder
            .matches
            .iter()
            .skip(window_start)
            .take(visible_rows)
            .filter_map(|found| finder.items.get(found.item).map(|item| (found, item)))
            .map(|(found, item)| {
                let mut line = matched_path_line(
                    &item.label,
                    &found.emphasis,
                    columns[0].width.saturating_sub(3) as usize,
                    app.theme.foreground,
                    app.theme.accent,
                );
                if !item.detail.is_empty() {
                    line.spans.push(Span::styled(
                        format!("  {}", item.detail),
                        Style::default().fg(app.theme.muted),
                    ));
                }
                ListItem::new(line)
            })
            .collect()
    };
    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.selection)
                .bold(),
        )
        .highlight_symbol("▸ ");
    let selected = (!finder.matches.is_empty()).then_some(finder.selected - window_start);
    let mut state = ListState::default().with_selected(selected);
    StatefulWidget::render(list, columns[0], frame.buffer_mut(), &mut state);

    if show_preview {
        let preview = finder.selected_preview().map_or_else(
            || vec![Line::from("No preview")],
            |preview| {
                preview
                    .split('\n')
                    .map(|line| Line::from(line.to_owned()))
                    .collect()
            },
        );
        let title = finder
            .selected_item()
            .map_or("Preview", |item| match item.kind {
                crate::finder::ResourceKind::Buffer => "Contents",
                crate::finder::ResourceKind::Terminal => "Output",
            });
        frame.render_widget(
            Paragraph::new(preview)
                .block(
                    Block::default()
                        .borders(Borders::LEFT)
                        .border_style(Style::default().fg(app.theme.muted))
                        .title(format!(" {title} ")),
                )
                .style(Style::default().fg(app.theme.foreground)),
            columns[1],
        );
    }
}

fn fuzzy_preview_lines(
    query: &str,
    lines: &[String],
    start_row: usize,
    focus_row: usize,
    emphasis: &[usize],
    theme: &TuiTheme,
) -> Vec<Line<'static>> {
    let line_digits = (start_row + lines.len()).max(1).to_string().len();
    let match_background = if crate::file_picker::is_direct_match(emphasis, query) {
        theme.fuzzy_match_primary
    } else {
        theme.fuzzy_match_secondary
    };
    lines
        .iter()
        .enumerate()
        .map(|(offset, line)| {
            let row = start_row + offset;
            let focused = row == focus_row;
            let prefix = format!(
                "{} {:>line_digits$} │ ",
                if focused { '›' } else { ' ' },
                row + 1
            );
            let mut spans = vec![Span::styled(
                prefix,
                Style::default().fg(if focused { theme.accent } else { theme.muted }),
            )];
            spans.extend(line.chars().enumerate().map(|(position, character)| {
                let style = if focused && emphasis.contains(&position) {
                    Style::default().fg(theme.foreground).bg(match_background)
                } else {
                    Style::default().fg(theme.foreground)
                };
                Span::styled(character.to_string(), style)
            }));
            Line::from(spans)
        })
        .collect()
}

fn fuzzy_matched_text_lines(
    query: &str,
    lines: &[String],
    emphasis: &[usize],
    theme: &TuiTheme,
) -> Vec<Line<'static>> {
    let emphasized = emphasis
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let match_background = if crate::file_picker::is_direct_match(emphasis, query) {
        theme.fuzzy_match_primary
    } else {
        theme.fuzzy_match_secondary
    };
    let mut offset = 0;
    lines
        .iter()
        .map(|line| {
            let spans = line
                .chars()
                .enumerate()
                .map(|(position, character)| {
                    let style = if emphasized.contains(&(offset + position)) {
                        Style::default().fg(theme.foreground).bg(match_background)
                    } else {
                        Style::default().fg(theme.foreground)
                    };
                    Span::styled(character.to_string(), style)
                })
                .collect::<Vec<_>>();
            offset += line.chars().count() + 1;
            Line::from(spans)
        })
        .collect()
}

fn matched_path_line(
    path: &str,
    positions: &[usize],
    width: usize,
    normal: ratatui::style::Color,
    accent: ratatui::style::Color,
) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    let characters = path.chars().collect::<Vec<_>>();
    let total_width = characters
        .iter()
        .map(|character| character.width().unwrap_or(0))
        .sum::<usize>();
    let truncated = total_width > width;
    let start = if truncated {
        let budget = width.saturating_sub(1);
        let mut used = 0;
        let mut start = characters.len();
        for (index, character) in characters.iter().enumerate().rev() {
            let cells = character.width().unwrap_or(0);
            if used + cells > budget {
                break;
            }
            used += cells;
            start = index;
        }
        start
    } else {
        0
    };
    let mut spans = Vec::new();
    if truncated {
        spans.push(Span::styled("…", Style::default().fg(normal)));
    }
    for (index, character) in characters.into_iter().enumerate().skip(start) {
        spans.push(Span::styled(
            character.to_string(),
            Style::default().fg(if positions.contains(&index) {
                accent
            } else {
                normal
            }),
        ));
    }
    Line::from(spans)
}

/// Symbols, references, diagnostics, and code actions.
fn draw_list(frame: &mut Frame<'_>, app: &TuiApp<'_>, editor_area: Rect) {
    let Some(picker) = &app.list else {
        return;
    };
    let setting = app.setting_choices_open();
    let preview_layout = picker.has_preview();
    let area = to_tui_rect(if setting {
        setting_choice_popup_area(editor_area)
    } else if preview_layout {
        centered(editor_area, 90, 85, 28, 8)
    } else {
        centered(editor_area, 86, 80, 24, 6)
    });
    if area.width < 3 || area.height < 3 {
        return;
    }
    let filter = if !picker.accepts_filter_input() {
        String::new()
    } else if picker.filter.is_empty() {
        "type to filter".to_owned()
    } else {
        format!("filter: {}", picker.filter)
    };
    let mut hints = Vec::new();
    if !filter.is_empty() {
        hints.push(filter);
    }
    if picker.has_tags() {
        hints.push(format!("Tab {}", picker.tag_label()));
    }
    if let Some(action) = &picker.primary_action {
        hints.push(format!("Enter {action}"));
    }
    if let Some((key, action)) = &picker.secondary_action {
        hints.push(format!("{key} {action}"));
    }
    if preview_layout {
        hints.push("Ctrl-t preview".to_owned());
    }
    if picker.purpose == crate::picker::ListPurpose::Report {
        hints.push("↑/↓ scroll".to_owned());
    }
    hints.push(if picker.purpose == crate::picker::ListPurpose::Report {
        "Esc dismiss".to_owned()
    } else {
        "Esc cancel".to_owned()
    });
    let title = format!(" {} · {} ", picker.title, hints.join(" · "));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.accent))
        .title(title)
        .style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.overlay_background),
        );
    let inner = block.inner(area);
    let show_preview = preview_layout && picker.show_preview && inner.width >= 72;
    let columns = if show_preview {
        TuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
            .split(inner)
    } else {
        TuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(inner)
    };
    let visible = picker.visible_indices();
    let report = picker.purpose == crate::picker::ListPurpose::Report;
    let report_offset = if report {
        picker.report_offset.min(visible.len().saturating_sub(1))
    } else {
        0
    };
    let displayed = visible
        .iter()
        .skip(report_offset)
        .take(if report {
            usize::from(columns[0].height).max(1)
        } else {
            usize::MAX
        })
        .copied()
        .collect::<Vec<_>>();
    let items = if visible.is_empty() {
        vec![
            ListItem::new(if report {
                "No report entries"
            } else {
                "No matching results"
            })
            .style(Style::default().fg(app.theme.muted)),
        ]
    } else {
        displayed
            .iter()
            .filter_map(|index| picker.items.get(*index))
            .map(|item| {
                // A dormant row keeps its shape and its place and gives up
                // only its colours, so the list still reads as one column of
                // names rather than two kinds of row. The selected row is
                // exempt: `highlight_style` repaints it anyway, and the
                // reader has to be able to read what they are about to act
                // on. `label_color` names the identifier column of the plain
                // layout; `text_color` is everything else, which is the
                // detail there and the whole label in the preview layout.
                let (label_color, text_color) = if item.is_dimmed() {
                    (app.theme.jump_text_muted, app.theme.jump_text_muted)
                } else {
                    (app.theme.accent, app.theme.foreground)
                };
                if preview_layout {
                    let emphasized = picker
                        .item_label_emphasis(item)
                        .into_iter()
                        .collect::<std::collections::HashSet<_>>();
                    ListItem::new(Line::from(
                        item.label
                            .chars()
                            .enumerate()
                            .map(|(position, character)| {
                                let style = if emphasized.contains(&position) {
                                    Style::default().fg(app.theme.accent).bold()
                                } else {
                                    Style::default().fg(text_color)
                                };
                                Span::styled(character.to_string(), style)
                            })
                            .collect::<Vec<_>>(),
                    ))
                } else {
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("{:<40}", short_identifier(&item.label, 39)),
                            Style::default().fg(label_color),
                        ),
                        Span::styled(item.detail.clone(), Style::default().fg(text_color)),
                    ]))
                }
            })
            .collect()
    };
    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.selection)
                .bold(),
        )
        .highlight_symbol("▸ ");
    let selected = (!report && !visible.is_empty())
        .then_some(picker.selected.min(visible.len().saturating_sub(1)));
    let mut state = ListState::default().with_selected(selected);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    StatefulWidget::render(list, columns[0], frame.buffer_mut(), &mut state);
    if show_preview {
        let preview = picker.selected_preview().map_or_else(
            || vec![Line::from("No preview")],
            |preview| {
                let lines = preview.split('\n').map(str::to_owned).collect::<Vec<_>>();
                fuzzy_matched_text_lines(
                    &picker.filter,
                    &lines,
                    &picker.selected_preview_emphasis(),
                    &app.theme,
                )
            },
        );
        frame.render_widget(
            Paragraph::new(preview)
                .block(
                    Block::default()
                        .borders(Borders::LEFT)
                        .title(format!(" {} ", picker.preview_title().unwrap_or("Preview"))),
                )
                .style(Style::default().fg(app.theme.foreground)),
            columns[1],
        );
    }
    if app.buffer_action_menu.is_some() {
        draw_buffer_actions(frame, app, editor_area);
    }
}

fn draw_buffer_actions(frame: &mut Frame<'_>, app: &TuiApp<'_>, editor_area: Rect) {
    let Some(menu) = &app.buffer_action_menu else {
        return;
    };
    let area = to_tui_rect(centered(
        editor_area,
        42,
        25,
        24,
        (menu.actions.len() as u16).saturating_add(2),
    ));
    if area.width < 3 || area.height < 3 {
        return;
    }
    let title = app.buffers.get(menu.buffer).map_or_else(
        || " Buffer actions ".to_owned(),
        |buffer| format!(" {} ", short_identifier(&buffer.display_name(), 30)),
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.accent))
        .title(title)
        .style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.overlay_background),
        );
    let items = menu
        .actions
        .iter()
        .map(|action| ListItem::new(action.label()))
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.selection)
                .bold(),
        )
        .highlight_symbol("▸ ");
    let mut state = ListState::default().with_selected(Some(menu.selected));
    frame.render_widget(Clear, area);
    StatefulWidget::render(list, area, frame.buffer_mut(), &mut state);
}

/// The completion popup, anchored below the caret and flipped above it when
/// there is no room.
fn draw_completion(
    frame: &mut Frame<'_>,
    app: &TuiApp<'_>,
    snapshot: &EditorSnapshot,
    editor_area: Rect,
) {
    let Some(state) = &app.completion else {
        return;
    };
    let visible = state.visible_indices();
    if visible.is_empty() {
        return;
    }
    let rows = visible.len().min(8);
    let width = editor_area.width.clamp(16, 64);
    let Some(area) = anchored(app, snapshot, editor_area, width, rows as u16 + 2) else {
        return;
    };
    let items = visible
        .iter()
        .filter_map(|index| state.items.get(*index))
        .map(|item| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<24}", short_identifier(&item.label, 23)),
                    Style::default().fg(app.theme.foreground),
                ),
                Span::styled(
                    format!("{:<14}", item.kind),
                    Style::default().fg(app.theme.accent),
                ),
                Span::styled(
                    short_identifier(&item.detail, 24),
                    Style::default().fg(app.theme.muted),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    // Every source can open on its own — Word for any three-character
    // prefix, Language after `.`/`:`, Path after `/` — so only Tab accepts;
    // Enter is reserved for its usual newline everywhere. Language is named
    // explicitly since "Complete" alone reads as this editor's own word
    // index rather than the attached server's answer.
    let title = match state.source {
        CompletionSource::Language => "LSP Complete",
        CompletionSource::Path | CompletionSource::Word => "Complete",
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.accent))
                .title(format!(" {title} · ↑/↓ Ctrl-n/p · Tab accept ")),
        )
        .style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.overlay_background),
        )
        .highlight_style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.selection)
                .bold(),
        );
    let mut list_state =
        ListState::default().with_selected(Some(state.selected.min(visible.len() - 1)));
    frame.render_widget(Clear, area);
    StatefulWidget::render(list, area, frame.buffer_mut(), &mut list_state);
}

fn draw_signature(
    frame: &mut Frame<'_>,
    app: &TuiApp<'_>,
    snapshot: &EditorSnapshot,
    editor_area: Rect,
) {
    let Some(state) = &app.signature else {
        return;
    };
    if app
        .completion
        .as_ref()
        .is_some_and(|completion| !completion.visible_indices().is_empty())
    {
        // Both popups anchor to the caret; the one being driven wins.
        return;
    }
    let lines = state
        .signatures
        .iter()
        .map(|signature| {
            // The active parameter is emphasised in place rather than listed
            // separately, which is the only presentation that survives a
            // one-line popup.
            match signature.active_parameter {
                Some((start, end))
                    if start <= end
                        && (end as usize) <= signature.label.len()
                        && signature.label.is_char_boundary(start as usize)
                        && signature.label.is_char_boundary(end as usize) =>
                {
                    Line::from(vec![
                        Span::styled(
                            signature.label[..start as usize].to_owned(),
                            Style::default().fg(app.theme.foreground),
                        ),
                        Span::styled(
                            signature.label[start as usize..end as usize].to_owned(),
                            Style::default().fg(app.theme.accent).bold(),
                        ),
                        Span::styled(
                            signature.label[end as usize..].to_owned(),
                            Style::default().fg(app.theme.foreground),
                        ),
                    ])
                }
                _ => Line::from(Span::styled(
                    signature.label.clone(),
                    Style::default().fg(app.theme.foreground),
                )),
            }
        })
        .collect::<Vec<_>>();
    let height = (lines.len() as u16 + 2).min(editor_area.height);
    let width = editor_area.width.clamp(16, 80);
    let Some(area) = anchored(app, snapshot, editor_area, width, height) else {
        return;
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(app.theme.muted))
                    .title(" Signature "),
            )
            .style(Style::default().bg(app.theme.overlay_background)),
        area,
    );
}

fn draw_hover(
    frame: &mut Frame<'_>,
    app: &TuiApp<'_>,
    snapshot: &EditorSnapshot,
    editor_area: Rect,
) {
    let Some(state) = &app.hover else {
        return;
    };
    let width = editor_area.width.clamp(16, 80);
    let rows = state.lines.len().min(12) as u16;
    let Some(area) = anchored(app, snapshot, editor_area, width, rows + 2) else {
        return;
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.accent));
    let visible_rows = usize::from(block.inner(area).height);
    let lines = state
        .lines
        .iter()
        .take(visible_rows)
        .map(|line| Line::from(line.clone()))
        .collect::<Vec<_>>();
    let omitted = state.lines.len().saturating_sub(visible_rows);
    let title = if omitted > 0 {
        format!(" Documentation · {omitted} more · Enter full view · other key dismisses ")
    } else {
        " Documentation · any key dismisses and continues ".to_owned()
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(block.title(title)).style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.overlay_background),
        ),
        area,
    );
}

/// A popup rectangle placed under the caret, flipped above it when the space
/// below is too small, and clamped inside the editor area.
fn anchored(
    app: &TuiApp<'_>,
    snapshot: &EditorSnapshot,
    editor_area: Rect,
    width: u16,
    height: u16,
) -> Option<TuiRect> {
    let pane = snapshot.pane(app.active_pane)?;
    if editor_area.width < 4 || editor_area.height < 4 {
        return None;
    }
    let bottom = editor_area.y.saturating_add(editor_area.height);
    // Every step saturates. A caret can be off-screen for a frame after a jump
    // or a resize, and a popup is never worth a panic.
    let screen_row = pane.cursor_screen_row.unwrap_or(0);
    let caret_y = pane
        .area
        .y
        .saturating_add(1)
        .saturating_add(screen_row.min(u16::MAX as usize) as u16)
        .min(bottom.saturating_sub(1));
    let height = height.min(editor_area.height);
    let width = width.min(editor_area.width);
    let below = caret_y.saturating_add(1);
    let y = if below.saturating_add(height) <= bottom {
        below
    } else {
        caret_y.saturating_sub(height)
    }
    .clamp(editor_area.y, bottom.saturating_sub(1));
    let x = pane.area.x.saturating_add(1).clamp(
        editor_area.x,
        editor_area
            .x
            .saturating_add(editor_area.width)
            .saturating_sub(width)
            .max(editor_area.x),
    );
    Some(TuiRect::new(x, y, width, height.min(bottom - y)))
}

fn short_identifier(identifier: &str, limit: usize) -> String {
    if identifier.chars().count() <= limit {
        return identifier.to_owned();
    }
    let keep = limit.saturating_sub(1);
    format!("{}…", identifier.chars().take(keep).collect::<String>())
}

fn draw_command_palette(frame: &mut Frame<'_>, app: &TuiApp<'_>, editor_area: Rect) {
    if editor_area.width < 3 || editor_area.height < 3 {
        return;
    }
    if let Some(hints) = app.matching_path_hints() {
        draw_command_path_hints(frame, app, editor_area, &hints);
        return;
    }
    let matches = app.matching_commands();
    let content_height = matches.len().max(1) as u16;
    let height = content_height
        .saturating_add(2)
        .min(editor_area.height)
        .min(24);
    let width = editor_area.width.min(100);
    let area = TuiRect::new(
        editor_area.x,
        editor_area.y + editor_area.height.saturating_sub(height),
        width,
        height,
    );

    let items = if matches.is_empty() {
        vec![ListItem::new("No matching commands").style(Style::default().fg(app.theme.muted))]
    } else {
        matches
            .iter()
            .map(|matched| {
                let others = matched.other_names();
                let aliases = if others.is_empty() {
                    String::new()
                } else {
                    format!("  aliases: {}", others.join(", "))
                };
                let available = matched.availability.is_available();
                let primary = if available {
                    app.theme.accent
                } else {
                    app.theme.muted
                };
                let foreground = if available {
                    app.theme.foreground
                } else {
                    app.theme.muted
                };
                let unavailable = matched
                    .availability
                    .reason()
                    .map_or_else(String::new, |reason| format!("  unavailable: {reason}"));
                let availability_label = if available {
                    String::new()
                } else {
                    "[unavailable] ".to_owned()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("[{:<13}] ", matched.category.label()),
                        Style::default().fg(app.theme.muted),
                    ),
                    Span::styled(
                        format!(":{:<24}", matched.usage()),
                        Style::default().fg(primary),
                    ),
                    Span::styled(availability_label, Style::default().fg(app.theme.muted)),
                    Span::styled(matched.spec.description, Style::default().fg(foreground)),
                    Span::styled(aliases, Style::default().fg(app.theme.muted)),
                    Span::styled(unavailable, Style::default().fg(app.theme.muted)),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.accent))
                .title(" Commands by category · ↑/↓ select · Tab complete "),
        )
        .style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.overlay_background),
        )
        .highlight_style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.selection)
                .bold(),
        )
        .highlight_symbol("▸ ");
    let selected =
        (!matches.is_empty()).then_some(app.command_selection.min(matches.len().saturating_sub(1)));
    let mut state = ListState::default().with_selected(selected);
    frame.render_widget(Clear, area);
    StatefulWidget::render(list, area, frame.buffer_mut(), &mut state);
}

fn draw_command_path_hints(
    frame: &mut Frame<'_>,
    app: &TuiApp<'_>,
    editor_area: Rect,
    hints: &[crate::app::PathHint],
) {
    let height = (hints.len().max(1) as u16)
        .saturating_add(2)
        .min(editor_area.height)
        .min(24);
    let width = editor_area.width.min(100);
    let area = TuiRect::new(
        editor_area.x,
        editor_area.y + editor_area.height.saturating_sub(height),
        width,
        height,
    );
    let items = if hints.is_empty() {
        vec![ListItem::new("No matching paths").style(Style::default().fg(app.theme.muted))]
    } else {
        hints
            .iter()
            .map(|hint| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!(
                            "{:<10} ",
                            if hint.is_directory {
                                "directory"
                            } else {
                                "file"
                            }
                        ),
                        Style::default().fg(app.theme.muted),
                    ),
                    Span::styled(hint.value.clone(), Style::default().fg(app.theme.accent)),
                    Span::styled(
                        format!("  {}", hint.detail),
                        Style::default().fg(app.theme.muted),
                    ),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.accent))
                .title(" Paths · directories open as explorers · ↑/↓ select · Tab complete "),
        )
        .style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.overlay_background),
        )
        .highlight_style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.selection)
                .bold(),
        )
        .highlight_symbol("▸ ");
    let selected =
        (!hints.is_empty()).then_some(app.command_selection.min(hints.len().saturating_sub(1)));
    let mut state = ListState::default().with_selected(selected);
    frame.render_widget(Clear, area);
    StatefulWidget::render(list, area, frame.buffer_mut(), &mut state);
}

/// Recently chosen programs offered above the "open with" prompt.
///
/// Hints only: the prompt takes whatever is typed, and an empty cache simply
/// says so rather than standing in the way of a first choice.
fn draw_program_hints(frame: &mut Frame<'_>, app: &TuiApp<'_>, editor_area: Rect) {
    if editor_area.width < 3 || editor_area.height < 3 {
        return;
    }
    let choices = app.matching_program_choices();
    let height = (choices.len().max(1) as u16)
        .saturating_add(2)
        .min(editor_area.height)
        .min(12);
    let width = editor_area.width.min(60);
    let area = TuiRect::new(
        editor_area.x,
        editor_area.y + editor_area.height.saturating_sub(height),
        width,
        height,
    );

    let items = if choices.is_empty() {
        vec![
            ListItem::new("No matching programs · type one and press Enter")
                .style(Style::default().fg(app.theme.muted)),
        ]
    } else {
        choices
            .iter()
            .map(|choice| {
                let suffix = match (choice.is_default, choice.system) {
                    (true, true) => "  default · system opener",
                    (true, false) => "  default",
                    (false, true) => "  system opener",
                    (false, false) => "",
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        choice.program.clone(),
                        Style::default().fg(app.theme.foreground),
                    ),
                    Span::styled(suffix, Style::default().fg(app.theme.muted)),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.accent))
                .title(" Open with · Enter open · ↑/↓ select · Tab actions "),
        )
        .style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.overlay_background),
        )
        .highlight_style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.selection)
                .bold(),
        )
        .highlight_symbol("▸ ");
    let selected =
        (!choices.is_empty()).then_some(app.command_selection.min(choices.len().saturating_sub(1)));
    let mut state = ListState::default().with_selected(selected);
    frame.render_widget(Clear, area);
    StatefulWidget::render(list, area, frame.buffer_mut(), &mut state);
}

fn draw_program_actions(frame: &mut Frame<'_>, app: &TuiApp<'_>, parent: Rect) {
    let Some(menu) = &app.program_action_menu else {
        return;
    };
    let width = parent.width.saturating_sub(4).clamp(3, 44);
    let height = (menu.actions.len() as u16)
        .saturating_add(2)
        .min(parent.height)
        .max(3);
    let area = TuiRect::new(
        parent.x + parent.width.saturating_sub(width) / 2,
        parent.y + parent.height.saturating_sub(height) / 2,
        width,
        height,
    );
    let items = menu
        .actions
        .iter()
        .map(|action| ListItem::new(action.label()))
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.accent))
                .title(format!(
                    " {} · Enter select · Tab/Esc back ",
                    menu.choice.program
                )),
        )
        .style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.overlay_background),
        )
        .highlight_style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.selection)
                .bold(),
        )
        .highlight_symbol("▸ ");
    let selected = Some(menu.selected.min(menu.actions.len().saturating_sub(1)));
    let mut state = ListState::default().with_selected(selected);
    frame.render_widget(Clear, area);
    StatefulWidget::render(list, area, frame.buffer_mut(), &mut state);
}

/// Width of the key column shared by the hint popup and the help window, so a
/// sequence and its description never run together.
const KEY_COLUMN_WIDTH: usize = 12;

fn draw_key_hints(
    frame: &mut Frame<'_>,
    app: &TuiApp<'_>,
    key_hints: &KeyHintState,
    editor_area: Rect,
) {
    if editor_area.width < 3 || editor_area.height < 3 {
        return;
    }

    if let Some(message) = key_hints.message() {
        let area = TuiRect::new(
            editor_area.x,
            editor_area.y + editor_area.height.saturating_sub(3),
            editor_area.width,
            3.min(editor_area.height),
        );
        let popup = Paragraph::new(message)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(app.theme.error))
                    .title(" Key hints "),
            )
            .style(
                Style::default()
                    .fg(app.theme.error)
                    .bg(app.theme.overlay_background),
            );
        frame.render_widget(Clear, area);
        frame.render_widget(popup, area);
        return;
    }

    let mode = app.key_hint_mode().unwrap_or(app.mode);
    let scope = app.key_binding_scope();
    let capabilities = app.command_capabilities();
    let mut rows = key_hints.rows_in(app.keymap(), mode, scope);
    for row in &mut rows {
        row.apply_capabilities(&capabilities);
    }
    if rows.is_empty() {
        return;
    }

    const MIN_COLUMN_WIDTH: usize = 36;
    const MAX_COLUMN_WIDTH: usize = 72;
    const COLUMN_GAP: usize = 2;
    const MAX_POPUP_HEIGHT: usize = 16;
    const MAX_KEY_COLUMN_WIDTH: usize = 20;
    let inner_width = editor_area.width.saturating_sub(2) as usize;
    // Size both columns from the widest entry so sequences stay aligned and
    // descriptions are not cut off.
    let key_width = rows
        .iter()
        .map(|row| key_hint_keys(row).chars().count())
        .max()
        .unwrap_or(KEY_COLUMN_WIDTH)
        .clamp(KEY_COLUMN_WIDTH, MAX_KEY_COLUMN_WIDTH);
    let widest = rows
        .iter()
        .map(|row| key_width + 1 + key_hint_description(row).chars().count())
        .max()
        .unwrap_or(MIN_COLUMN_WIDTH)
        .clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH)
        + COLUMN_GAP;
    let columns = (inner_width / widest).max(1).min(rows.len());
    let maximum_content_rows = (editor_area.height as usize)
        .saturating_sub(2)
        .min(MAX_POPUP_HEIGHT.saturating_sub(2))
        .max(1);
    let capacity = maximum_content_rows * columns;
    let maximum_offset = rows.len().saturating_sub(capacity);
    key_hints.note_scroll_limit(maximum_offset);
    let offset = key_hints.scroll_offset();
    let visible = &rows[offset..(offset + capacity).min(rows.len())];
    let content_rows = visible.len().div_ceil(columns).max(1);
    let height = (content_rows + 2).min(editor_area.height as usize) as u16;
    let area = TuiRect::new(
        editor_area.x,
        editor_area.y + editor_area.height.saturating_sub(height),
        editor_area.width,
        height,
    );
    let range = if rows.len() > visible.len() {
        let arrows_are_free =
            key_hints.scrolls_with_arrow_in(KeyCode::Up, mode, scope, app.keymap())
                && key_hints.scrolls_with_arrow_in(KeyCode::Down, mode, scope, app.keymap());
        let scroll_keys = if arrows_are_free {
            "↑/↓"
        } else {
            "Alt-j/k"
        };
        format!(
            " {}-{}/{} {scroll_keys}",
            offset + 1,
            offset + visible.len(),
            rows.len()
        )
    } else {
        String::new()
    };
    let sequence = if key_hints.is_pending() {
        format!("{} …", key_hints.display_pending())
    } else {
        rows[0].sequence.to_string()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.accent))
        .title(format!(" Keys: {sequence}{range} "))
        .style(Style::default().bg(app.theme.overlay_background));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    let column_width = (inner.width as usize / columns).max(1) as u16;
    for (index, row) in visible.iter().enumerate() {
        let column = index / content_rows;
        let row_index = index % content_rows;
        let x = inner.x + column as u16 * column_width;
        let width = if column + 1 == columns {
            inner.right().saturating_sub(x)
        } else {
            column_width
        };
        let cell = TuiRect::new(x, inner.y + row_index as u16, width, 1);
        frame.render_widget(Paragraph::new(key_hint_line(row, app, key_width)), cell);
    }
}

/// The description column of a hint row, including any availability suffix.
fn key_hint_description(row: &KeyHintRow) -> String {
    let exact = if row.exact { "  (exact)" } else { "" };
    let namespace = if row.namespace { " ›" } else { "" };
    if let Some(reason) = &row.unavailable_reason {
        return format!(
            "{}{namespace}{exact}  unavailable: {reason}",
            row.description
        );
    }
    match row.availability {
        BindingAvailability::Implemented => format!("{}{namespace}{exact}", row.description),
        BindingAvailability::Planned(reason) => {
            format!("{}{namespace}{exact}  planned: {reason}", row.description)
        }
        BindingAvailability::Unsupported(reason) => {
            format!(
                "{}{namespace}{exact}  unsupported: {reason}",
                row.description
            )
        }
    }
}

/// The key column of a hint row: the sequence that opened this menu, then any
/// other spelling of the same command. Both belong in the key column rather
/// than in the prose, which is already the longest thing on the line.
fn key_hint_keys(row: &KeyHintRow) -> String {
    match row.alias.as_ref() {
        Some(alias) => match row.alias_modes {
            Some(modes) => {
                let modes = modes
                    .iter()
                    .map(|mode| mode.label())
                    .collect::<Vec<_>>()
                    .join("/");
                format!("{}, {modes} {alias}", row.sequence)
            }
            None => format!("{}, {alias}", row.sequence),
        },
        None => row.sequence.to_string(),
    }
}

fn key_hint_line(row: &KeyHintRow, app: &TuiApp<'_>, key_width: usize) -> Line<'static> {
    let unavailable = !row.availability.is_implemented() || row.unavailable_reason.is_some();
    let description_style = Style::default()
        .fg(if unavailable {
            app.theme.muted
        } else {
            app.theme.foreground
        })
        .add_modifier(if unavailable {
            Modifier::DIM
        } else {
            Modifier::empty()
        });
    let exact = if row.exact { "  (exact)" } else { "" };
    let namespace = if row.namespace { " ›" } else { "" };
    let availability = match (&row.unavailable_reason, row.availability) {
        (Some(reason), BindingAvailability::Implemented) => {
            format!("  unavailable: {reason}")
        }
        (_, BindingAvailability::Implemented) => String::new(),
        (_, BindingAvailability::Planned(reason)) => format!("  planned: {reason}"),
        (_, BindingAvailability::Unsupported(reason)) => format!("  unsupported: {reason}"),
    };
    let key_style = if unavailable {
        description_style
    } else {
        Style::default().fg(app.theme.accent)
    };
    Line::from(vec![
        Span::styled(format!("{:<key_width$} ", key_hint_keys(row)), key_style),
        Span::styled(row.description, description_style),
        Span::styled(
            namespace,
            if unavailable {
                description_style
            } else {
                Style::default().fg(app.theme.accent)
            },
        ),
        Span::styled(exact, Style::default().fg(app.theme.muted)),
        Span::styled(availability, description_style),
    ])
}

fn centered(area: Rect, width_percent: u16, height_percent: u16, min_w: u16, min_h: u16) -> Rect {
    // Widened before scaling, as `layout::split_extent` does. A terminal wide
    // enough to overflow the product is unusual but reachable, and in a debug
    // build the overflow is a panic that takes unsaved buffers with it.
    let scale = |extent: u16, percent: u16| {
        (u32::from(extent) * u32::from(percent) / 100).min(u32::from(u16::MAX)) as u16
    };
    let width = scale(area.width, width_percent).max(min_w).min(area.width);
    let height = scale(area.height, height_percent)
        .max(min_h)
        .min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn overlay_action_hints(overlay: &OverlaySnapshot) -> String {
    overlay
        .actions
        .iter()
        .map(|action| format!("{} {}", action.key_hint, action.label))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Confirmation windows carry a few sentences rather than a navigable list,
/// so size them from that content instead of using the generic list popup's
/// percentage-based geometry.
fn confirmation_overlay_area(area: Rect, overlay: &OverlaySnapshot) -> Rect {
    const MIN_WIDTH: u16 = 28;
    const MAX_WIDTH: u16 = 88;

    let action_hints = overlay_action_hints(overlay);
    let title_width = overlay.title.width()
        + if action_hints.is_empty() {
            4
        } else {
            action_hints.width() + 7
        };
    let message_width = overlay
        .message
        .as_deref()
        .unwrap_or_default()
        .split('\n')
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or_default()
        + 2;
    let minimum = MIN_WIDTH.min(area.width);
    let maximum = MAX_WIDTH.min(area.width);
    let width = u16::try_from(title_width.max(message_width))
        .unwrap_or(u16::MAX)
        .clamp(minimum, maximum);
    let inner_width = width.saturating_sub(2).max(1);
    let message_rows = overlay
        .message
        .as_deref()
        .map_or(1, |message| wrapped_text_rows(message, inner_width));
    let height = u16::try_from(message_rows.saturating_add(2))
        .unwrap_or(u16::MAX)
        .clamp(3.min(area.height), area.height);

    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn wrapped_text_rows(text: &str, width: u16) -> usize {
    let width = usize::from(width.max(1));
    text.split('\n')
        .map(|line| {
            let mut rows = 1usize;
            let mut used = 0usize;
            for word in line.split_whitespace() {
                let word_width = word.width();
                let separator = usize::from(used > 0);
                if separator + word_width <= width.saturating_sub(used) {
                    used += separator + word_width;
                    continue;
                }
                if used > 0 {
                    rows += 1;
                }
                rows += word_width.saturating_sub(1) / width;
                used = word_width.saturating_sub(1) % width + 1;
            }
            rows
        })
        .sum()
}

fn setting_popup_area(area: Rect) -> Rect {
    fixed_centered(area, 60, 9)
}

/// A setting's choice list, which unlike the typed prompt beside it shows
/// rows: tall enough to read most of the theme list without scrolling, and
/// wide enough that the title still names every key the list answers to.
fn setting_choice_popup_area(area: Rect) -> Rect {
    fixed_centered(area, 70, 14)
}

fn fixed_centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn to_tui_rect(rect: Rect) -> TuiRect {
    TuiRect::new(rect.x, rect.y, rect.width, rect.height)
}

fn from_tui_rect(rect: TuiRect) -> Rect {
    Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::{
        buffer::Position, config::Config, jump_labels::LabelPart, key_hints::KeyHintState,
        selection::Selection, snapshot::LongRunningActionSnapshot, text::Transaction,
    };

    #[test]
    fn long_running_action_uses_a_right_anchored_rotating_bar() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        // Command mode's caret is asserted through the style rather than a
        // frame: an open palette floats over the row the caret is on.
        assert_eq!(
            text_run_style(
                &theme,
                Mode::Command,
                TextRole::Caret,
                None,
                None,
                false,
                None,
                None,
                None,
            )
            .bg,
            Some(to_tui_color(app.theme.cursor_command))
        );
        let render_at = |elapsed_millis| {
            let mut terminal = Terminal::new(TestBackend::new(120, 1)).unwrap();
            terminal
                .draw(|frame| {
                    draw_long_running_action(
                        frame,
                        &theme,
                        &LongRunningActionSnapshot {
                            label: "Indexing workspace".to_owned(),
                            detail: "/project".to_owned(),
                            elapsed_millis,
                            cancel_hint: Some(":stop-index".to_owned()),
                        },
                        NotificationCounts::default(),
                        frame.area(),
                    );
                })
                .unwrap();
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
        };
        let frames = [0, 80, 160, 240, 320].map(render_at);
        let start = &frames[0];
        assert!(start.contains("Indexing workspace · /project · 0s · :stop-index"));
        for (line, spinner) in frames.iter().zip(['-', '\\', '|', '/', '-']) {
            assert_eq!(line.chars().last(), Some(spinner), "{line:?}");
            assert!(!line.contains('─'), "{line:?}");
            assert!(!line.contains('━'), "{line:?}");
        }

        assert_eq!(clip_with_ellipsis("e\u{301}xy", 2), "e\u{301}…");
        assert_eq!(clip_with_ellipsis("👩‍💻xy", 3), "👩‍💻…");

        let mut narrow = Terminal::new(TestBackend::new(20, 1)).unwrap();
        narrow
            .draw(|frame| {
                draw_long_running_action(
                    frame,
                    &theme,
                    &LongRunningActionSnapshot {
                        label: "Indéxing 👩‍💻".to_owned(),
                        detail: "/界".to_owned(),
                        elapsed_millis: 320,
                        cancel_hint: None,
                    },
                    NotificationCounts::default(),
                    frame.area(),
                );
            })
            .unwrap();
        assert!(
            narrow
                .backend()
                .buffer()
                .content
                .iter()
                .last()
                .is_some_and(|cell| cell.symbol() == "-")
        );
    }

    fn status_with_notifications(counts: NotificationCounts) -> StatusSnapshot {
        StatusSnapshot {
            workspace_number: None,
            mode: Mode::Normal,
            workspace_directory: "/a/very/long/workspace/directory".to_owned(),
            dirty: true,
            read_only: false,
            cursor: Position::new(41, 7),
            line_count: 100,
            selection_count: 1,
            lsp_summary: Some("rust-analyzer 0E 1W".to_owned()),
            git_summary: Some("main ~3".to_owned()),
            long_running_action: None,
            notification_counts: counts,
            interaction_line: String::new(),
            interaction_line_error: false,
            prompt_cursor_column: None,
        }
    }

    fn rendered_status_line(status: &StatusSnapshot, width: u16) -> String {
        rendered_status_line_for(status, SessionMode::Standalone, width)
    }

    fn rendered_status_line_for(
        status: &StatusSnapshot,
        session: SessionMode,
        width: u16,
    ) -> String {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| draw_normal_status(frame, &theme, status, session, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// Renders `draw_status`'s interaction line row (not the status row above
    /// it) at `width` columns, trimmed of the blank cells Ratatui pads a
    /// short line with.
    fn rendered_interaction_line(status: &StatusSnapshot, width: u16) -> String {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let mut terminal = Terminal::new(TestBackend::new(width, 2)).unwrap();
        terminal
            .draw(|frame| {
                let status_area = TuiRect::new(0, 0, width, 1);
                let interaction_area = TuiRect::new(0, 1, width, 1);
                draw_status(
                    frame,
                    &theme,
                    status,
                    SessionMode::Standalone,
                    status_area,
                    interaction_area,
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..width)
            .map(|x| buffer[(x, 1)].symbol())
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    #[test]
    fn normal_status_names_the_workspace_and_keeps_the_other_status_fields() {
        let mut status = status_with_notifications(NotificationCounts {
            errors: 1,
            warnings: 2,
            infos: 0,
        });
        status.workspace_directory = "/project/runyte".to_owned();
        status.selection_count = 3;

        let line = rendered_status_line(&status, 120);

        assert!(
            line.starts_with(" NOR │ standalone │ Workspace: /project/runyte [+]"),
            "{line:?}"
        );
        assert!(line.contains("42:8 · 41% │ 3 sel"), "{line:?}");
        assert!(line.contains("│ main ~3"), "{line:?}");
        assert!(line.contains("│ rust-analyzer 0E 1W"), "{line:?}");
        assert!(line.ends_with("│ E1 W2 "), "{line:?}");
    }

    #[test]
    fn only_the_status_mode_label_follows_the_caret_color() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        for mode in [Mode::Normal, Mode::Insert, Mode::Select, Mode::Command] {
            let mut status = status_with_notifications(NotificationCounts::default());
            status.mode = mode;
            let mut terminal = Terminal::new(TestBackend::new(120, 1)).unwrap();
            terminal
                .draw(|frame| {
                    draw_normal_status(
                        frame,
                        &theme,
                        &status,
                        SessionMode::Standalone,
                        frame.area(),
                    )
                })
                .unwrap();

            let expected = theme.cursor(mode);
            let cells = &terminal.backend().buffer().content;
            assert!(cells[..5].iter().all(|cell| cell.bg == expected));
            assert!(
                cells[5..].iter().all(|cell| cell.bg == theme.background),
                "{} changed the background beyond its mode label",
                mode.label()
            );
        }
    }

    /// The marker is the key somebody would press to come back here, so an
    /// unnumbered workspace shows nothing rather than a placeholder standing
    /// where a working shortcut would be.
    #[test]
    fn the_status_row_marks_a_numbered_workspace_and_omits_the_marker_otherwise() {
        let mut status = status_with_notifications(NotificationCounts::default());
        status.workspace_directory = "/project/runyte".to_owned();
        status.dirty = false;

        status.workspace_number = Some(1);
        let numbered = rendered_status_line_for(&status, SessionMode::Persistent, 120);
        assert!(
            numbered.starts_with(" NOR │ persistent │ [S1] Workspace: /project/runyte "),
            "{numbered:?}"
        );

        status.workspace_number = None;
        let unnumbered = rendered_status_line_for(&status, SessionMode::Persistent, 120);
        assert!(
            unnumbered.starts_with(" NOR │ persistent │ Workspace: /project/runyte "),
            "{unnumbered:?}"
        );
    }

    /// The two render paths draw the same snapshot, so the mode has to come
    /// from the frontend rather than from the frame: a host's own frame is
    /// standalone in its process and persistent in the one displaying it.
    #[test]
    fn the_status_row_names_the_workspace_mode_before_the_workspace() {
        let mut status = status_with_notifications(NotificationCounts::default());
        status.workspace_directory = "/project/runyte".to_owned();
        status.dirty = false;

        let standalone = rendered_status_line_for(&status, SessionMode::Standalone, 120);
        let persistent = rendered_status_line_for(&status, SessionMode::Persistent, 120);

        assert!(
            standalone.starts_with(" NOR │ standalone │ Workspace: /project/runyte "),
            "{standalone:?}"
        );
        assert!(
            persistent.starts_with(" NOR │ persistent │ Workspace: /project/runyte "),
            "{persistent:?}"
        );
    }

    #[test]
    fn interaction_line_echo_is_clipped_to_the_frames_width() {
        // The fixed shape ("p (Paste after the selection · failed: ") is 39
        // cells; at width 60 the trailing "x" message is cut to 18 of them
        // plus a marker, an exact, easy-to-check boundary.
        let mut status = status_with_notifications(NotificationCounts::default());
        status.interaction_line = format!(
            "p (Paste after the selection · failed: {})",
            "x".repeat(100)
        );

        let narrow = rendered_interaction_line(&status, 60);
        assert_eq!(
            narrow,
            format!(
                "p (Paste after the selection · failed: {}...",
                "x".repeat(18)
            )
        );

        let wide = rendered_interaction_line(&status, 200);
        assert_eq!(wide, status.interaction_line, "{wide:?}");
        assert!(!wide.ends_with("..."), "{wide:?}");
    }

    #[test]
    fn interaction_line_echo_keeps_only_its_first_line_when_clipped() {
        let mut status = status_with_notifications(NotificationCounts::default());
        status.interaction_line =
            "p (Paste after the selection · failed: first line\nsecond line)".to_owned();

        let line = rendered_interaction_line(&status, 120);
        assert_eq!(
            line, "p (Paste after the selection · failed: first line...)",
            "{line:?}"
        );
    }

    #[test]
    fn an_active_prompt_is_never_clipped_with_a_marker() {
        // The prompt's cursor column is computed against the untruncated
        // string, so the rendered text must stay untruncated too, even past
        // the frame's width, rather than gain a `...` the cursor math does
        // not account for.
        let mut status = status_with_notifications(NotificationCounts::default());
        status.interaction_line = format!("search: {}", "x".repeat(80));
        status.prompt_cursor_column = Some(8);

        let line = rendered_interaction_line(&status, 40);
        assert!(!line.ends_with("..."), "{line:?}");
        assert!(line.starts_with("search: xxxx"), "{line:?}");
    }

    #[test]
    fn narrow_status_trims_the_start_of_a_long_unicode_workspace_path() {
        let mut status = status_with_notifications(NotificationCounts::default());
        status.workspace_directory = "/discarded/prefix/母/e\u{301}/workspace".to_owned();
        status.dirty = false;
        status.git_summary = None;
        status.lsp_summary = None;

        // Wide enough to leave the path the same budget it had before the
        // session role joined the prefix, so this still clips where a wide
        // character would be split rather than somewhere easier.
        let line = rendered_status_line(&status, 62);

        assert!(
            line.starts_with(" NOR │ standalone │ Workspace: .../母 "),
            "{line:?}"
        );
        assert!(line.ends_with(" 42:8 · 41% "), "{line:?}");
        assert!(!line.contains("discarded"), "{line:?}");
        assert_eq!(
            clip_path_start("/discarded/母/e\u{301}/workspace", 18),
            ".../母/e\u{301}/workspace"
        );
        assert_eq!(clip_path_start("/workspace", 3), "...");
        assert_eq!(clip_path_start("/workspace", 2), "");
    }

    #[test]
    fn pane_title_leaves_a_short_path_untouched() {
        let title = pane_title_text("[file] /tmp/x.rs", false, false, None, 40);
        assert_eq!(title, " [file] /tmp/x.rs ");
    }

    #[test]
    fn pane_title_trims_a_long_path_from_the_start_keeping_markers() {
        let name = "[file] /home/user/code/runyte/src/very/deeply/nested/module.rs";
        let title = pane_title_text(name, true, true, None, 40);

        assert!(title.starts_with(" ..."), "{title:?}");
        assert!(title.ends_with("module.rs [+] [RO] "), "{title:?}");
        // The title text occupies the pane's top border line inside its two
        // corner cells, so it fills the pane width minus those two borders.
        assert_eq!(UnicodeWidthStr::width(title.as_str()), 40 - 2);
    }

    #[test]
    fn pane_title_degrades_gracefully_when_too_narrow_for_an_ellipsis() {
        let title = pane_title_text("[file] /a/b/c.rs", false, false, None, 2);
        assert_eq!(title, "  ");
    }

    #[test]
    fn pane_title_names_the_maximized_view_only_while_one_is_active() {
        assert_eq!(
            pane_title_text(
                "[file] /tmp/x.rs",
                false,
                false,
                Some(MaximizedView::Zen),
                40
            ),
            " [file] /tmp/x.rs [zen] "
        );
        assert_eq!(
            pane_title_text(
                "[file] /tmp/x.rs",
                false,
                false,
                Some(MaximizedView::Fullscreen),
                40
            ),
            " [file] /tmp/x.rs [fullscreen] "
        );
        assert_eq!(
            pane_title_text("[file] /tmp/x.rs", false, false, None, 40),
            " [file] /tmp/x.rs "
        );
    }

    #[test]
    fn pane_title_keeps_the_maximized_tag_when_the_path_is_trimmed() {
        let name = "[file] /home/user/code/runyte/src/very/deeply/nested/module.rs";
        let title = pane_title_text(name, true, true, Some(MaximizedView::Zen), 40);

        assert!(title.starts_with(" ..."), "{title:?}");
        assert!(title.ends_with("module.rs [+] [RO] [zen] "), "{title:?}");
        assert_eq!(UnicodeWidthStr::width(title.as_str()), 40 - 2);
    }

    #[test]
    fn rendered_status_tracks_the_directory_selected_by_cd() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-status-workspace-{}-{unique}",
            std::process::id()
        ));
        let before = root.join("before");
        let after = root.join("café");
        std::fs::create_dir_all(&before).unwrap();
        std::fs::create_dir_all(&after).unwrap();
        let mut app = App::new(Config::default(), None).unwrap();
        app.working_directory = before;

        app.handle_key(crate::input::KeyStroke::char(':')).unwrap();
        for character in "cd ../café".chars() {
            app.handle_key(crate::input::KeyStroke::char(character))
                .unwrap();
        }
        app.handle_key(crate::input::KeyStroke::plain(crate::input::KeyCode::Enter))
            .unwrap();

        let geometry = frame_geometry(TuiRect::new(0, 0, 160, 8));
        let prepared = app.prepare_view(geometry);
        let snapshot = app.snapshot(&prepared);
        assert_eq!(snapshot.status.workspace_directory, after.to_string_lossy());
        let screen = rendered(&mut app, 160, 8);
        assert!(
            screen.contains(&format!("Workspace: {}", after.to_string_lossy())),
            "{screen:?}"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn compact_notification_indicator_is_right_anchored_at_narrow_widths() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let status = status_with_notifications(NotificationCounts {
            errors: 1,
            warnings: 2,
            infos: 1,
        });
        let mut terminal = Terminal::new(TestBackend::new(20, 1)).unwrap();
        terminal
            .draw(|frame| {
                draw_normal_status(
                    frame,
                    &theme,
                    &status,
                    SessionMode::Standalone,
                    frame.area(),
                )
            })
            .unwrap();

        let cells = &terminal.backend().buffer().content;
        let line = cells.iter().map(|cell| cell.symbol()).collect::<String>();
        assert!(line.ends_with(" │ E4 "), "{line:?}");
        let error_cell = cells
            .iter()
            .find(|cell| cell.symbol() == "E")
            .expect("compact error indicator");
        assert_eq!(error_cell.fg, theme.error);
    }

    #[test]
    fn long_running_action_keeps_unread_notifications_visible() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let mut terminal = Terminal::new(TestBackend::new(48, 1)).unwrap();
        terminal
            .draw(|frame| {
                draw_long_running_action(
                    frame,
                    &theme,
                    &LongRunningActionSnapshot {
                        label: "Pushing".to_owned(),
                        detail: "main".to_owned(),
                        elapsed_millis: 2_000,
                        cancel_hint: Some(":git-cancel".to_owned()),
                    },
                    NotificationCounts {
                        errors: 0,
                        warnings: 2,
                        infos: 0,
                    },
                    frame.area(),
                );
            })
            .unwrap();

        let cells = &terminal.backend().buffer().content;
        let line = cells.iter().map(|cell| cell.symbol()).collect::<String>();
        assert!(line.ends_with("\\ │ W2 "), "{line:?}");
        assert!(line.ends_with(" │ W2 "), "{line:?}");
        let warning_cell = cells
            .iter()
            .find(|cell| cell.symbol() == "W")
            .expect("warning indicator");
        assert_eq!(warning_cell.fg, theme.warning);
    }

    fn render_test_frame(frame: &mut Frame<'_>, app: &mut App, hints: &KeyHintState) {
        let prepared = app.prepare_view(frame_geometry(frame.area()));
        let snapshot = app.snapshot(&prepared);
        render(frame, app, &snapshot, hints);
    }

    /// Every percentage the popups actually use, across widths that straddle
    /// the point where the old `u16` product overflowed (713 cells at 92%,
    /// 729 at 90%).
    #[test]
    fn centered_popups_scale_on_terminals_too_wide_for_a_u16_product() {
        for extent in [1, 80, 712, 713, 728, 729, 2000, u16::MAX] {
            for percent in [80, 85, 86, 88, 90, 92] {
                let area = Rect {
                    x: 0,
                    y: 0,
                    width: extent,
                    height: extent,
                };
                let rect = centered(area, percent, percent, 1, 1);

                assert!(rect.width <= extent, "{extent}x{percent} widened the area");
                assert!(rect.height <= extent, "{extent}x{percent} grew the area");
                let expected =
                    (u32::from(extent) * u32::from(percent) / 100).clamp(1, u32::from(extent));
                assert_eq!(u32::from(rect.width), expected, "{extent} at {percent}%");
                assert!(rect.x + rect.width <= extent);
                assert!(rect.y + rect.height <= extent);
            }
        }
    }

    #[test]
    fn setting_popups_keep_a_typed_value_compact_and_give_a_choice_list_room() {
        let editor = Rect {
            x: 4,
            y: 2,
            width: 120,
            height: 40,
        };
        assert_eq!(
            setting_popup_area(editor),
            Rect {
                x: 34,
                y: 17,
                width: 60,
                height: 9,
            }
        );
        assert_eq!(
            setting_choice_popup_area(editor),
            Rect {
                x: 29,
                y: 15,
                width: 70,
                height: 14,
            }
        );
    }

    /// The theme list's title carries the most hints of any setting popup.
    /// It has to fit inside the border, or Ratatui truncates the one naming
    /// the key that gets back out of the list.
    #[test]
    fn the_choice_popup_is_wide_enough_for_its_longest_title() {
        let editor = Rect {
            x: 0,
            y: 0,
            width: 200,
            height: 60,
        };
        let title = " theme · type to filter · Tab light · Enter to save · Esc cancel ";
        assert!(
            title.width() <= usize::from(setting_choice_popup_area(editor).width - 2),
            "the setting choice popup cannot show its own key hints"
        );
    }

    /// A terminal smaller than the popup gets the terminal, not an overflow.
    #[test]
    fn setting_popups_never_exceed_the_area_they_are_centered_in() {
        let editor = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 6,
        };
        assert_eq!(setting_popup_area(editor), editor);
        assert_eq!(setting_choice_popup_area(editor), editor);
    }

    #[test]
    fn runyte_colors_convert_exhaustively_at_the_tui_boundary() {
        use ratatui::style::Color as TuiColor;

        let cases = [
            (RunyteColor::Reset, TuiColor::Reset),
            (RunyteColor::Black, TuiColor::Black),
            (RunyteColor::Red, TuiColor::Red),
            (RunyteColor::Green, TuiColor::Green),
            (RunyteColor::Yellow, TuiColor::Yellow),
            (RunyteColor::Blue, TuiColor::Blue),
            (RunyteColor::Magenta, TuiColor::Magenta),
            (RunyteColor::Cyan, TuiColor::Cyan),
            (RunyteColor::White, TuiColor::White),
            (RunyteColor::Gray, TuiColor::Gray),
            (RunyteColor::DarkGray, TuiColor::DarkGray),
            (RunyteColor::Rgb(1, 2, 3), TuiColor::Rgb(1, 2, 3)),
        ];

        for (runyte, tui) in cases {
            assert_eq!(to_tui_color(runyte), tui);
        }
    }

    #[test]
    fn cross_mode_key_hint_aliases_name_their_mode() {
        let mut hints = KeyHintState::default();
        hints.observe(
            crate::input::KeyStroke::char(' '),
            Mode::Normal,
            crate::keymap::default_keymap(),
        );
        hints.observe(
            crate::input::KeyStroke::char('l'),
            Mode::Normal,
            crate::keymap::default_keymap(),
        );
        let completion = hints
            .rows(crate::keymap::default_keymap(), Mode::Normal)
            .into_iter()
            .find(|row| {
                row.target
                    == Some(crate::keymap::BindingTarget::Editor(
                        crate::command::EditorCommand::TriggerCompletion,
                    ))
            })
            .expect("the language namespace lists completion");

        assert_eq!(key_hint_keys(&completion), "Space l c, INS Ctrl-x");
    }

    #[test]
    fn folded_row_uses_an_accent_colored_gutter_chevron_without_growing_it() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let pane = PaneSnapshot {
            pane_id: 0,
            area: Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 6,
            },
            body: Rect {
                x: 1,
                y: 1,
                width: 38,
                height: 4,
            },
            active: true,
            jump_active: false,
            dimmed: false,
            drawable: true,
            title: crate::snapshot::PaneTitle {
                name: "fold.rs".to_owned(),
                dirty: false,
                read_only: false,
                maximized: None,
            },
            line_numbers: true,
            line_digits: 3,
            signs: false,
            changes: false,
            text_width: 33,
            gutter_width: 0,
            content_indent: 0,
            scroll_row: 0,
            scroll_wrap: 0,
            wrap_width: 33,
            cursor_screen_row: Some(0),
            rows: Vec::new(),
            terminal: None,
        };
        let row = SnapshotRow::Text(crate::snapshot::VisibleRow {
            document_row: 135,
            continuation: false,
            folded: true,
            cursor_row: true,
            diagnostic_sign: None,
            change: None,
            diff: None,
            compared: None,
            notification_severity: None,
            runs: Vec::new(),
        });

        let line = snapshot_line(&theme, Mode::Normal, &pane, &row);
        assert_eq!(line.spans[0].content, "136");
        assert_eq!(line.spans[1].content, "▸");
        assert_eq!(line.spans[1].style.fg, Some(theme.accent));
        assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(line.spans[2].content, "│");
        assert_eq!(line.spans[3].content, " ");
        assert_eq!(line.spans.iter().map(|span| span.width()).sum::<usize>(), 6);
    }

    /// Each kind of change has its own one-cell symbol and theme colour.
    #[test]
    fn git_change_symbols_are_one_cell_and_use_the_change_palette() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let pane = PaneSnapshot {
            pane_id: 0,
            area: Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 6,
            },
            body: Rect {
                x: 1,
                y: 1,
                width: 38,
                height: 4,
            },
            active: true,
            jump_active: false,
            dimmed: false,
            drawable: true,
            title: crate::snapshot::PaneTitle {
                name: "changed.rs".to_owned(),
                dirty: true,
                read_only: false,
                maximized: None,
            },
            line_numbers: false,
            line_digits: 0,
            signs: false,
            changes: true,
            text_width: 38,
            gutter_width: 0,
            content_indent: 0,
            scroll_row: 0,
            scroll_wrap: 0,
            wrap_width: 38,
            cursor_screen_row: Some(0),
            rows: Vec::new(),
            terminal: None,
        };
        let mark = |change| {
            let row = SnapshotRow::Text(crate::snapshot::VisibleRow {
                document_row: 0,
                continuation: false,
                folded: false,
                cursor_row: false,
                diagnostic_sign: None,
                change,
                diff: None,
                compared: None,
                notification_severity: None,
                runs: Vec::new(),
            });
            let line = snapshot_line(&theme, Mode::Normal, &pane, &row);
            assert_eq!(line.spans.len(), 1);
            assert_eq!(line.spans[0].width(), 1, "the column is exactly one cell");
            (
                line.spans[0].content.to_string(),
                line.spans[0].style.fg.unwrap(),
            )
        };

        assert_eq!(
            mark(Some(LineChange::Added)),
            ("+".to_owned(), theme.change_added)
        );
        assert_eq!(
            mark(Some(LineChange::Modified)),
            ("~".to_owned(), theme.change_modified)
        );
        assert_eq!(
            mark(Some(LineChange::RemovedAbove)),
            ("-".to_owned(), theme.change_removed)
        );
        assert_eq!(
            mark(Some(LineChange::RemovedBelow)),
            ("-".to_owned(), theme.change_removed)
        );
        assert_eq!(mark(None).0, " ", "an unchanged row still holds the column");
    }

    #[test]
    fn git_change_symbols_share_the_pre_separator_wrap_column() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let pane = PaneSnapshot {
            pane_id: 0,
            area: Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 6,
            },
            body: Rect {
                x: 1,
                y: 1,
                width: 38,
                height: 4,
            },
            active: true,
            jump_active: false,
            dimmed: false,
            drawable: true,
            title: crate::snapshot::PaneTitle {
                name: "changed.rs".to_owned(),
                dirty: true,
                read_only: false,
                maximized: None,
            },
            line_numbers: true,
            line_digits: 2,
            signs: false,
            changes: true,
            text_width: 33,
            gutter_width: 5,
            content_indent: 0,
            scroll_row: 0,
            scroll_wrap: 0,
            wrap_width: 33,
            cursor_screen_row: Some(0),
            rows: Vec::new(),
            terminal: None,
        };
        let render = |continuation| {
            snapshot_line(
                &theme,
                Mode::Normal,
                &pane,
                &SnapshotRow::Text(crate::snapshot::VisibleRow {
                    document_row: 6,
                    continuation,
                    folded: false,
                    cursor_row: false,
                    diagnostic_sign: None,
                    change: Some(LineChange::Added),
                    diff: None,
                    compared: None,
                    notification_severity: None,
                    runs: Vec::new(),
                }),
            )
        };

        let line = render(false);
        assert_eq!(
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<Vec<_>>(),
            [" 7", "+", "│", " "]
        );
        assert_eq!(line.spans[1].style.fg, Some(theme.change_added));
        assert_eq!(line.width(), pane.gutter_width);

        let continuation = render(true);
        assert_eq!(
            continuation
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<Vec<_>>(),
            ["  ", "↪", "│", " "]
        );
        assert_eq!(continuation.width(), pane.gutter_width);
    }

    #[test]
    fn a_changed_fold_anchor_gets_separate_fold_and_change_columns() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let pane = PaneSnapshot {
            pane_id: 0,
            area: Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 6,
            },
            body: Rect {
                x: 1,
                y: 1,
                width: 38,
                height: 4,
            },
            active: true,
            jump_active: false,
            dimmed: false,
            drawable: true,
            title: crate::snapshot::PaneTitle {
                name: "changed.rs".to_owned(),
                dirty: true,
                read_only: false,
                maximized: None,
            },
            line_numbers: true,
            line_digits: 2,
            signs: false,
            changes: true,
            text_width: 32,
            gutter_width: 6,
            content_indent: 0,
            scroll_row: 0,
            scroll_wrap: 0,
            wrap_width: 32,
            cursor_screen_row: Some(0),
            rows: Vec::new(),
            terminal: None,
        };
        let row = SnapshotRow::Text(crate::snapshot::VisibleRow {
            document_row: 6,
            continuation: false,
            folded: true,
            cursor_row: false,
            diagnostic_sign: None,
            change: Some(LineChange::Modified),
            diff: None,
            compared: None,
            notification_severity: None,
            runs: Vec::new(),
        });

        let line = snapshot_line(&theme, Mode::Normal, &pane, &row);
        assert_eq!(
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<Vec<_>>(),
            [" 7", "▸", "~", "│", " "]
        );
        assert_eq!(line.spans[1].style.fg, Some(theme.accent));
        assert_eq!(line.spans[2].style.fg, Some(theme.change_modified));
        assert_eq!(line.width(), pane.gutter_width);
    }

    /// A patch is read by its leading characters, and the colours are the
    /// gutter's, so added text is the same green wherever it is met.
    #[test]
    fn diff_rows_are_coloured_by_what_the_line_is() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let pane = PaneSnapshot {
            pane_id: 0,
            area: Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 6,
            },
            body: Rect {
                x: 1,
                y: 1,
                width: 38,
                height: 4,
            },
            active: true,
            jump_active: false,
            dimmed: false,
            drawable: true,
            title: crate::snapshot::PaneTitle {
                name: "[git diff a.rs]".to_owned(),
                dirty: false,
                read_only: false,
                maximized: None,
            },
            line_numbers: false,
            line_digits: 0,
            signs: false,
            changes: false,
            text_width: 38,
            gutter_width: 0,
            content_indent: 0,
            scroll_row: 0,
            scroll_wrap: 0,
            wrap_width: 38,
            cursor_screen_row: None,
            rows: Vec::new(),
            terminal: None,
        };
        let colour = |diff, text: &str| {
            let row = SnapshotRow::Text(crate::snapshot::VisibleRow {
                document_row: 0,
                continuation: false,
                folded: false,
                cursor_row: false,
                diagnostic_sign: None,
                change: None,
                diff,
                compared: None,
                notification_severity: None,
                runs: vec![crate::snapshot::TextRun {
                    text: text.to_owned(),
                    kind: TextRunKind::Text {
                        role: TextRole::Plain,
                        scope: None,
                        diagnostic: None,
                        directory: false,
                        count: None,
                    },
                }],
            });
            snapshot_line(&theme, Mode::Normal, &pane, &row).spans[0]
                .style
                .fg
                .unwrap()
        };

        assert_eq!(colour(Some(DiffLine::Added), "+new"), theme.change_added);
        assert_eq!(
            colour(Some(DiffLine::Removed), "-old"),
            theme.change_removed
        );
        assert_eq!(colour(Some(DiffLine::Hunk), "@@ -1 +1 @@"), theme.accent);
        assert_eq!(colour(Some(DiffLine::Meta), "index a..b"), theme.muted);
        assert_eq!(colour(None, " context"), theme.foreground);
    }

    /// The changed-file list's two counts are painted from the theme, in the
    /// colours it already gives an added and a removed line, so a theme that
    /// is not red and green anywhere else is not red and green here either.
    #[test]
    fn count_runs_use_the_theme_change_colours() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let colour = |count| {
            text_run_style(
                &theme,
                Mode::Normal,
                TextRole::Plain,
                None,
                None,
                false,
                count,
                None,
                None,
            )
            .fg
        };

        assert_eq!(colour(Some(CountKind::Added)), Some(theme.change_added));
        assert_eq!(colour(Some(CountKind::Removed)), Some(theme.change_removed));
        assert_eq!(
            colour(None),
            Some(theme.foreground),
            "the rest of the row is ordinary text"
        );
    }

    #[test]
    fn directory_runs_use_the_theme_directory_color() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        assert_eq!(
            text_run_style(
                &theme,
                Mode::Normal,
                TextRole::Plain,
                None,
                None,
                true,
                None,
                None,
                None,
            )
            .fg,
            Some(theme.directory)
        );
        assert_eq!(
            text_run_style(
                &theme,
                Mode::Normal,
                TextRole::Plain,
                None,
                None,
                false,
                None,
                None,
                None,
            )
            .fg,
            Some(theme.foreground)
        );
    }

    /// Editor/application state that frame preparation and drawing must not
    /// change. Pane rectangles and scroll/wrap fields belong to the separate
    /// `ViewFingerprint` because preparation owns those mutations.
    #[derive(Debug, Eq, PartialEq)]
    struct SemanticFingerprint {
        config: String,
        theme: String,
        theme_name: String,
        buffers: String,
        syntax_languages: Vec<Option<String>>,
        registry_errors: Vec<String>,
        panes: Vec<(usize, usize, Selection, String, Option<usize>)>,
        layout: String,
        active_pane: usize,
        mode: Mode,
        command: String,
        command_cursor: usize,
        command_selection: usize,
        prompt_kind: PromptKind,
        external_target: Option<std::path::PathBuf>,
        host_preferences: String,
        overlays: String,
        diagnostics: String,
        status: String,
        status_error: bool,
        should_quit: bool,
        roots: Vec<std::path::PathBuf>,
        jump: String,
    }

    impl SemanticFingerprint {
        fn capture(app: &App) -> Self {
            let mut panes = app
                .panes
                .iter()
                .map(|(id, pane)| {
                    (
                        *id,
                        pane.buffer,
                        pane.selection.clone(),
                        format!("{:?}", pane.jumps),
                        pane.directory_buffer,
                    )
                })
                .collect::<Vec<_>>();
            panes.sort_by_key(|(id, ..)| *id);

            Self {
                config: format!("{:?}", app.config),
                theme: format!("{:?}", app.theme),
                theme_name: app.theme_name.clone(),
                buffers: format!("{:?}", app.buffers),
                syntax_languages: app
                    .syntax
                    .iter()
                    .map(|syntax| {
                        syntax
                            .as_ref()
                            .map(|syntax| app.registry.language_name(syntax.language()).to_owned())
                    })
                    .collect(),
                registry_errors: app
                    .registry
                    .errors()
                    .into_iter()
                    .map(|error| error.to_string())
                    .collect(),
                panes,
                layout: format!("{:?}", app.layout),
                active_pane: app.active_pane,
                mode: app.mode,
                command: app.command.clone(),
                command_cursor: app.command_cursor,
                command_selection: app.command_selection,
                prompt_kind: app.prompt_kind,
                external_target: app.external_target.clone(),
                host_preferences: format!("{:?}", app.programs),
                overlays: format!(
                    "{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
                    app.picker,
                    app.fs_confirmation,
                    app.list,
                    app.buffer_action_menu,
                    app.completion,
                    app.signature,
                    app.hover,
                ),
                diagnostics: format!("{:?}", app.diagnostics),
                status: app.status.clone(),
                status_error: app.status_error,
                should_quit: app.should_quit,
                roots: vec![
                    app.working_directory.clone(),
                    app.project_root.clone(),
                    app.state_root.clone(),
                ],
                jump: format!("{:?}", app.jump),
            }
        }
    }

    /// Layout and pane viewport state owned by `App::prepare_view`.
    #[derive(Debug, Eq, PartialEq)]
    struct ViewFingerprint {
        areas: Vec<(usize, Rect)>,
        panes: Vec<(usize, usize, usize, usize, usize, bool)>,
    }

    impl ViewFingerprint {
        fn capture(app: &App) -> Self {
            let mut areas = app
                .areas
                .iter()
                .map(|(id, area)| (*id, *area))
                .collect::<Vec<_>>();
            areas.sort_by_key(|(id, _)| *id);
            let mut panes = app
                .panes
                .iter()
                .map(|(id, pane)| {
                    (
                        *id,
                        pane.scroll_row,
                        pane.scroll_wrap,
                        pane.scroll_col,
                        pane.wrap_width,
                        pane.preserve_scroll,
                    )
                })
                .collect::<Vec<_>>();
            panes.sort_by_key(|(id, ..)| *id);
            Self { areas, panes }
        }
    }

    fn assert_preparation_is_idempotent_and_render_is_immutable(
        app: &mut App,
        width: u16,
        height: u16,
    ) {
        let semantic_before = SemanticFingerprint::capture(app);
        let view_before = ViewFingerprint::capture(app);
        let geometry = frame_geometry(TuiRect::new(0, 0, width, height));

        let prepared = app.prepare_view(geometry);
        let snapshot = app.snapshot(&prepared);

        assert_eq!(
            SemanticFingerprint::capture(app),
            semantic_before,
            "view preparation changed editor/application state"
        );
        let prepared_view = ViewFingerprint::capture(app);
        assert_ne!(
            prepared_view, view_before,
            "fixture did not exercise view preparation"
        );

        let repeated = app.prepare_view(geometry);
        let repeated_snapshot = app.snapshot(&repeated);

        assert_eq!(repeated, prepared, "preparation was not idempotent");
        assert_eq!(
            repeated_snapshot, snapshot,
            "snapshot construction was not repeatable"
        );
        assert_eq!(
            SemanticFingerprint::capture(app),
            semantic_before,
            "repeated preparation changed editor/application state"
        );
        assert_eq!(
            ViewFingerprint::capture(app),
            prepared_view,
            "repeated preparation changed view state"
        );

        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let hints = KeyHintState::default();
        for pass in 1..=2 {
            terminal
                .draw(|frame| render(frame, app, &snapshot, &hints))
                .unwrap();
            assert_eq!(
                SemanticFingerprint::capture(app),
                semantic_before,
                "render pass {pass} changed editor/application state"
            );
            assert_eq!(
                ViewFingerprint::capture(app),
                prepared_view,
                "render pass {pass} changed prepared view state"
            );
        }
    }

    #[test]
    fn renders_editor_and_status_into_test_backend() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new(Config::default(), None).unwrap();

        let hints = KeyHintState::default();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();

        let screen = terminal.backend().buffer();
        let rendered: String = screen
            .content
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(rendered.contains("[scratch]"));
        assert!(rendered.contains("NOR"));
    }

    #[test]
    fn attached_key_hints_dim_rows_the_host_marks_unavailable() {
        let mut host =
            crate::workspace::WorkspaceHost::new(App::new(Config::default(), None).unwrap());
        let mut hints = KeyHintState::default();
        hints.observe(
            crate::input::KeyStroke::char(' '),
            Mode::Normal,
            host.app().keymap(),
        );
        let frame = host.prepare_frame_with_hints(
            crate::app::FrameGeometry {
                screen: Rect {
                    x: 0,
                    y: 0,
                    width: 160,
                    height: 30,
                },
                editor: Rect {
                    x: 0,
                    y: 0,
                    width: 160,
                    height: 28,
                },
                status: Rect {
                    x: 0,
                    y: 28,
                    width: 160,
                    height: 1,
                },
                message: Rect {
                    x: 0,
                    y: 29,
                    width: 160,
                    height: 1,
                },
            },
            Some(&hints),
        );
        let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
        terminal
            .draw(|terminal_frame| render_host_frame(terminal_frame, &frame))
            .unwrap();

        let screen = terminal.backend().buffer();
        let label = "Language (LSP)";
        let cell = (0..screen.area.height)
            .find_map(|row| {
                let text = (0..screen.area.width)
                    .map(|column| screen[(column, row)].symbol())
                    .collect::<String>();
                text.find(label).map(|column| (column as u16, row))
            })
            .expect("the attached Space menu renders the LSP namespace");
        assert!(screen[cell].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn the_status_line_carries_progress_beside_the_cursor_and_no_theme_name() {
        let config = Config {
            theme: Some("gruvbox".into()),
            ..Config::default()
        };
        let mut app = App::new(config, None).unwrap();
        let content = (0..101)
            .map(|row| format!("row-{row:03}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.buffers[0].apply(&Transaction::insert(0, &content));
        let offset = app.buffers[0].line_to_offset(25);
        app.panes.get_mut(&0).unwrap().selection = crate::selection::Selection::point(offset);

        let screen = rendered(&mut app, 80, 24);

        assert!(screen.contains("26:1 · 25%"), "{screen:?}");
        assert!(
            !screen.contains("gruvbox"),
            "the theme name belongs to Space o t, not the status line: {screen:?}"
        );
    }

    #[test]
    fn first_frame_keeps_a_text_margin_after_the_line_number_separator() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "hello"));

        let screen = rendered(&mut app, 20, 6);

        assert!(screen.contains("1 │ hello"), "{screen:?}");
    }

    #[test]
    fn editor_caret_uses_the_theme_color_for_each_mode() {
        let mut config = Config::default();
        config.editor.line_numbers = false;
        config.theme = Some("light".into());
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "x"));
        let mut terminal = Terminal::new(TestBackend::new(20, 6)).unwrap();
        let hints = KeyHintState::default();

        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        let normal_cell = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .find(|cell| cell.symbol() == "x")
            .unwrap();
        assert_eq!(
            normal_cell.style().bg,
            Some(to_tui_color(app.theme.cursor_normal))
        );

        app.handle_key(crate::input::KeyStroke::new(
            crate::input::KeyCode::Char('i'),
            crate::input::Modifiers::NONE,
        ))
        .unwrap();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        let insert_cell = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .find(|cell| cell.symbol() == "x")
            .unwrap();
        assert_eq!(
            insert_cell.style().bg,
            Some(to_tui_color(app.theme.cursor_insert))
        );

        app.handle_key(crate::input::KeyStroke::new(
            crate::input::KeyCode::Escape,
            crate::input::Modifiers::NONE,
        ))
        .unwrap();
        app.handle_key(crate::input::KeyStroke::new(
            crate::input::KeyCode::Char('v'),
            crate::input::Modifiers::NONE,
        ))
        .unwrap();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        let select_cell = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .find(|cell| cell.symbol() == "x")
            .unwrap();
        assert_eq!(
            select_cell.style().bg,
            Some(to_tui_color(app.theme.cursor_select))
        );

        let theme = TuiTheme::new(&app.theme);
        assert_eq!(
            text_run_style(
                &theme,
                Mode::Select,
                TextRole::PrimarySelected,
                None,
                None,
                false,
                None,
                None,
                None,
            )
            .bg,
            Some(to_tui_color(app.theme.selection_primary))
        );
        assert_eq!(
            text_run_style(
                &theme,
                Mode::Select,
                TextRole::PrimaryCaret,
                None,
                None,
                false,
                None,
                None,
                None,
            )
            .bg,
            Some(to_tui_color(app.theme.cursor_select))
        );
        assert_eq!(
            text_run_style(
                &theme,
                Mode::Select,
                TextRole::ReplaceCaret,
                None,
                None,
                false,
                None,
                None,
                None,
            )
            .bg,
            Some(to_tui_color(app.theme.cursor_insert))
        );

        let mut insert_multi = App::new(Config::default(), None).unwrap();
        insert_multi.buffers[0].apply(&Transaction::insert(0, "x\nx"));
        insert_multi
            .handle_key(crate::input::KeyStroke::char('C'))
            .unwrap();
        insert_multi
            .handle_key(crate::input::KeyStroke::char('i'))
            .unwrap();
        terminal
            .draw(|frame| render_test_frame(frame, &mut insert_multi, &hints))
            .unwrap();
        let insert_carets = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .filter(|cell| cell.symbol() == "x")
            .collect::<Vec<_>>();
        assert_eq!(insert_carets.len(), 2);
        assert!(insert_carets.iter().all(|cell| {
            cell.style().bg == Some(to_tui_color(insert_multi.theme.cursor_insert))
        }));
    }

    #[cfg(unix)]
    #[test]
    fn explorer_symlinks_render_a_muted_hint_beside_their_names() {
        let directory = std::env::temp_dir().join(format!(
            "runyte-ui-symlink-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("true_file.txt"), "text").unwrap();
        std::os::unix::fs::symlink("true_file.txt", directory.join("file.txt")).unwrap();
        let mut app = App::new(Config::default(), Some(directory.clone())).unwrap();
        let muted = to_tui_color(app.theme.muted);
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &KeyHintState::default()))
            .unwrap();

        let screen = terminal.backend().buffer();
        let rendered: String = screen
            .content
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(rendered.contains("file.txt  → true_file.txt"), "{rendered}");
        let arrow = screen
            .content
            .iter()
            .find(|cell| cell.symbol() == "→")
            .unwrap();
        assert_eq!(arrow.style().fg, Some(muted));

        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn command_mode_renders_the_filterable_command_palette() {
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new(Config::default(), None).unwrap();
        app.handle_key(crate::input::KeyStroke::new(
            crate::input::KeyCode::Char(':'),
            crate::input::Modifiers::NONE,
        ))
        .unwrap();

        let hints = KeyHintState::default();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();

        let screen = terminal.backend().buffer();
        let rendered: String = screen
            .content
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("Open file explorer in the working directory"));

        for character in "theme".chars() {
            app.handle_key(crate::input::KeyStroke::char(character))
                .unwrap();
        }
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect::<String>();
        assert!(rendered.contains("Choose and save the editor theme"));
    }

    /// The screen position of `needle`'s first cell, searched row by row.
    fn find_text(buffer: &ratatui::buffer::Buffer, needle: &str) -> Option<(u16, u16)> {
        let needle = needle.chars().collect::<Vec<_>>();
        (0..buffer.area.height).find_map(|y| {
            let row = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect::<Vec<_>>();
            row.windows(needle.len())
                .position(|window| window == needle)
                .map(|x| (x as u16, y))
        })
    }

    /// An overlay floats on a ground one step off the pane's, through the
    /// standalone widgets and through the snapshot path an attached client
    /// draws alike, so neither session type shows a popup cut out of its text.
    #[test]
    fn overlays_paint_on_a_ground_of_their_own() {
        let mut app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        assert_ne!(theme.overlay_background, theme.background);
        app.handle_key(crate::input::KeyStroke::char(':')).unwrap();

        // Tall enough that the bottom-anchored palette leaves pane rows above
        // it to compare against.
        let hints = KeyHintState::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let (x, y) = find_text(buffer, "Commands by category").expect("the palette title");
        // The row under the title is the selected one, which carries the
        // selection colour; the one below it shows the popup's own ground.
        assert_eq!(
            buffer[(x, y + 2)].bg,
            theme.overlay_background,
            "an unselected palette row"
        );
        assert_eq!(buffer[(x, 0)].bg, theme.background, "the pane behind it");

        let overlay = app
            .overlay_snapshots()
            .into_iter()
            .find(|overlay| overlay.kind == OverlayKind::CommandPalette)
            .expect("the palette also has a snapshot");
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        terminal
            .draw(|frame| {
                let prepared = app.prepare_view(frame_geometry(frame.area()));
                let snapshot = app.snapshot(&prepared);
                for pane in &snapshot.panes {
                    draw_pane(frame, &theme, snapshot.mode, pane);
                }
                draw_snapshot_overlay(frame, &theme, &overlay, &snapshot);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let (x, y) = find_text(buffer, &overlay.title).expect("the snapshot overlay title");
        assert_eq!(
            buffer[(x, y + 2)].bg,
            theme.overlay_background,
            "an unselected snapshot overlay row"
        );
        assert_eq!(buffer[(x, 0)].bg, theme.background, "the pane behind it");
    }

    #[test]
    fn inactive_panes_use_the_ground_between_the_active_pane_and_overlays() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "active pane\n"));
        app.handle_key(crate::input::KeyStroke::ctrl('w')).unwrap();
        app.handle_key(crate::input::KeyStroke::char('v')).unwrap();

        let theme = TuiTheme::new(&app.theme);
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        let hints = KeyHintState::default();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        let prepared = app.prepare_view(frame_geometry(TuiRect::new(0, 0, 100, 24)));
        let snapshot = app.snapshot(&prepared);
        assert_eq!(snapshot.panes.len(), 2);

        for pane in &snapshot.panes {
            let x = pane.body.x + pane.body.width.saturating_sub(1);
            let y = pane.body.y + pane.body.height.saturating_sub(1);
            let expected = theme.pane_background(pane.active);
            assert_eq!(
                terminal.backend().buffer()[(x, y)].bg,
                expected,
                "pane {} active={}",
                pane.pane_id,
                pane.active
            );
        }
        assert_ne!(theme.background, theme.inactive_background);
        assert_ne!(theme.inactive_background, theme.overlay_background);
    }

    #[test]
    fn terminal_default_cells_follow_their_panes_ground() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let cell = TerminalCell::default();

        assert_eq!(
            terminal_style(&theme, true, &cell).bg,
            Some(theme.background)
        );
        assert_eq!(
            terminal_style(&theme, false, &cell).bg,
            Some(theme.inactive_background)
        );
    }

    #[test]
    fn active_terminal_cursor_uses_the_theme_color_for_each_mode() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let cell = TerminalCell {
            character: 'x',
            foreground: crate::terminal::Color::Rgb(1, 2, 3),
            background: crate::terminal::Color::Rgb(4, 5, 6),
            attributes: crate::terminal::Attributes::REVERSE,
            ..TerminalCell::default()
        };
        let view = TerminalView {
            revision: 0,
            columns: 1,
            rows: vec![vec![cell]],
            line_ids: vec![None],
            cursor: Some((0, 0)),
            scrollback: 0,
            live: true,
            review: false,
            newer_output: false,
            highlights: Vec::new(),
        };

        for mode in [Mode::Normal, Mode::Insert, Mode::Select, Mode::Command] {
            let line = terminal_line(&theme, mode, true, false, &view, 0, &view.rows[0]);
            let style = line.spans[0].style;
            assert_eq!(
                style.fg,
                Some(theme.background),
                "{} foreground",
                mode.label()
            );
            assert_eq!(
                style.bg,
                Some(theme.cursor(mode)),
                "{} background",
                mode.label()
            );
            assert!(
                !style.add_modifier.contains(Modifier::REVERSED),
                "the child's reverse attribute masked the {} cursor",
                mode.label()
            );
        }
    }

    #[test]
    fn a_dimmed_terminal_grays_the_childs_colours_but_not_its_caret() {
        let app = App::new(Config::default(), None).unwrap();
        let theme = TuiTheme::new(&app.theme);
        let cell = |character: char| TerminalCell {
            character,
            foreground: crate::terminal::Color::Rgb(1, 2, 3),
            background: crate::terminal::Color::Rgb(4, 5, 6),
            ..TerminalCell::default()
        };
        let view = TerminalView {
            revision: 0,
            columns: 2,
            rows: vec![vec![cell('x'), cell('y')]],
            line_ids: vec![None],
            cursor: Some((0, 0)),
            scrollback: 0,
            live: true,
            review: false,
            newer_output: false,
            highlights: Vec::new(),
        };

        let line = terminal_line(&theme, Mode::Command, true, true, &view, 0, &view.rows[0]);
        // A terminal is pane content like any other, so a prompt grays the
        // child's own colours too.
        assert_eq!(line.spans[1].style.fg, Some(theme.jump_text_muted));
        // The caret is rebuilt after the dim and still names the mode.
        assert_eq!(line.spans[0].style.bg, Some(theme.cursor_command));

        let bright = terminal_line(&theme, Mode::Command, true, false, &view, 0, &view.rows[0]);
        assert_eq!(
            bright.spans[1].style.fg,
            Some(ratatui::style::Color::Rgb(1, 2, 3))
        );
    }

    #[cfg(unix)]
    #[test]
    fn list_manager_action_overlay_is_drawn_above_its_list() {
        let root = std::env::temp_dir().join(format!(
            "runyte-ui-workspace-actions-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let mut app = App::new(Config::default(), Some(root.clone())).unwrap();
        app.enable_persistent_session();
        app.execute(crate::command::parse_named_command("sl", None).unwrap())
            .unwrap();
        app.apply_workspace_event(crate::workspace::WorkspaceEvent::Refreshed {
            generation: 1,
            result: Ok(vec![crate::workspace::WorkspaceRow {
                number: None,
                id: "aaaaaaaaaaaaaaaa".to_owned(),
                name: Some("project".to_owned()),
                project_root: root.clone(),
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: None,
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: None,
            }]),
        });
        app.handle_key(crate::input::KeyStroke::plain(crate::input::KeyCode::Tab))
            .unwrap();

        let screen = rendered(&mut app, 90, 24);
        assert!(
            screen.contains("Attach to this persistent session"),
            "{screen}"
        );
        assert!(
            screen.contains("Change this persistent session"),
            "{screen}"
        );
        // A stopped row offers Forget, and neither way of stopping. The bare
        // `Close` label is too generic to test for, so each absent action is
        // named by its own description.
        assert!(
            screen.contains("Remove this stopped session's visited-history record"),
            "{screen}"
        );
        assert!(!screen.contains("Stop this persistent session"), "{screen}");
        assert!(!screen.contains("Force close"), "{screen}");
        assert!(
            !screen.contains("End protected buffers, waiters, and live terminals"),
            "{screen}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_session_list_grays_out_stopped_rows_and_leaves_running_ones_bright() {
        let root = std::env::temp_dir().join(format!(
            "runyte-ui-stopped-sessions-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        // The light theme is the one whose dimming role is a distinct colour
        // rather than a second name for `muted`.
        let config = Config {
            theme: Some("light".into()),
            ..Config::default()
        };
        let theme = config.resolve_theme("light").unwrap();
        let mut app = App::new(config, Some(root.clone())).unwrap();
        app.enable_persistent_session();
        app.execute(crate::command::parse_named_command("sl", None).unwrap())
            .unwrap();
        // The running row sorts first and keeps the selection, so the stopped
        // one below it is compared without the highlight in the way.
        app.apply_workspace_event(crate::workspace::WorkspaceEvent::Refreshed {
            generation: 1,
            result: Ok(vec![
                crate::workspace::WorkspaceRow {
                    number: None,
                    id: "aaaaaaaaaaaaaaaa".to_owned(),
                    name: Some("zzzz".to_owned()),
                    project_root: root.clone(),
                    running: true,
                    incompatible_protocol: None,
                    unsaved_buffers: None,
                    pending_wait_requests: None,
                    live_terminals: None,
                    terminal_sessions: None,
                    interactive_attached: None,
                },
                crate::workspace::WorkspaceRow {
                    number: None,
                    id: "bbbbbbbbbbbbbbbb".to_owned(),
                    name: Some("qqqq".to_owned()),
                    project_root: root.join("archive"),
                    running: false,
                    incompatible_protocol: None,
                    unsaved_buffers: None,
                    pending_wait_requests: None,
                    live_terminals: None,
                    terminal_sessions: None,
                    interactive_attached: None,
                },
            ]),
        });

        let hints = KeyHintState::default();
        const WIDTH: usize = 90;
        let mut terminal = Terminal::new(TestBackend::new(WIDTH as u16, 14)).unwrap();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        // Located by the row the name is drawn on rather than by the glyph
        // alone: the status line carries the real project path, which is not
        // this test's to choose.
        let cells = terminal.backend().buffer().content.clone();
        let row_colors = |name: &str| {
            let row = cells
                .chunks(WIDTH)
                .find(|row| {
                    row.iter()
                        .map(|cell| cell.symbol())
                        .collect::<String>()
                        .contains(name)
                })
                .unwrap_or_else(|| panic!("{name} should be listed"));
            let symbols = row
                .iter()
                .map(|cell| cell.symbol().to_owned())
                .collect::<Vec<_>>();
            let start = (0..symbols.len())
                .find(|index| {
                    symbols[*index..]
                        .iter()
                        .take(name.chars().count())
                        .cloned()
                        .collect::<String>()
                        == name
                })
                .unwrap();
            let label = row[start..start + name.chars().count()]
                .iter()
                .map(|cell| cell.style().fg)
                .collect::<Vec<_>>();
            let detail = row[start + name.chars().count()..]
                .iter()
                .filter(|cell| !cell.symbol().trim().is_empty() && cell.symbol() != "│")
                .map(|cell| cell.style().fg)
                .collect::<Vec<_>>();
            (label, detail)
        };

        let (running, running_detail) = row_colors("zzzz");
        assert!(
            running
                .iter()
                .chain(&running_detail)
                .all(|color| *color == Some(to_tui_color(theme.foreground))),
            "the selected running session keeps the highlight's full weight: \
             {running:?} {running_detail:?}"
        );
        let (stopped, stopped_detail) = row_colors("qqqq");
        assert!(!stopped_detail.is_empty());
        // Name and detail both recede, so the whole row reads as one dormant
        // line rather than a bright name with a grayed tail.
        assert!(
            stopped
                .iter()
                .chain(&stopped_detail)
                .all(|color| *color == Some(to_tui_color(theme.jump_text_muted))),
            "a stopped session uses the dimming role: {stopped:?} {stopped_detail:?}"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn command_palette_labels_categories_and_contextually_unavailable_rows() {
        let backend = TestBackend::new(100, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new(Config::default(), None).unwrap();
        app.handle_key(crate::input::KeyStroke::new(
            crate::input::KeyCode::Char(':'),
            crate::input::Modifiers::NONE,
        ))
        .unwrap();
        for character in "outline".chars() {
            app.handle_key(crate::input::KeyStroke::new(
                crate::input::KeyCode::Char(character),
                crate::input::Modifiers::NONE,
            ))
            .unwrap();
        }

        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &KeyHintState::default()))
            .unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(rendered.contains("[Syntax"));
        assert!(rendered.contains("[unavailable]"));
    }

    #[test]
    fn filesystem_plan_confirmation_lists_actions_and_safe_choices() {
        let root = std::env::temp_dir().join(format!(
            "runyte-ui-fs-plan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let snapshot = crate::fs_plan::DirectorySnapshot::read(&root).unwrap();
        let plan = crate::fs_plan::FsPlan::build(
            root.clone(),
            snapshot,
            vec![crate::fs_plan::DesiredEntry::create(
                "new-file",
                crate::fs_plan::EntryKind::File,
            )],
        )
        .unwrap();
        let mut app = App::new(Config::default(), None).unwrap();
        app.fs_confirmation = Some(crate::app::FsConfirmation {
            buffer: 0,
            plan,
            selected: 0,
        });

        let screen = rendered(&mut app, 100, 24);

        assert!(screen.contains("Filesystem plan"));
        assert!(screen.contains("create new-file"));
        assert!(screen.contains("trash deletes"));
        assert!(screen.contains("permanently delete"));
        assert!(!root.join("new-file").exists());
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn standalone_render_draws_the_shared_confirmation_overlay() {
        let root = std::env::temp_dir().join(format!(
            "runyte-ui-confirmation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("kept.txt"), "kept\n").unwrap();
        let mut app = App::new(Config::default(), Some(root.clone())).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "invented.txt\n"));
        let invocation = crate::command::parse_colon_command("reload").unwrap();

        assert!(matches!(
            app.execute(invocation).unwrap(),
            crate::app::CommandOutcome::Confirmation(_)
        ));
        let overlay = app
            .overlay_snapshots()
            .into_iter()
            .find(|overlay| overlay.kind == OverlayKind::Confirmation)
            .unwrap();
        let area = confirmation_overlay_area(
            Rect {
                x: 0,
                y: 0,
                width: 120,
                height: 22,
            },
            &overlay,
        );

        assert_eq!(area.height, 5, "three sentences plus the border");
        assert!(
            area.width < 80,
            "short confirmations stay compact: {area:?}"
        );
        let screen = rendered(&mut app, 120, 24);

        assert!(screen.contains("Discard directory edits"), "{screen:?}");
        assert!(screen.contains("Enter discard and refresh"), "{screen:?}");
        assert!(
            screen.contains("Discard unsaved directory edits"),
            "{screen:?}"
        );
        assert!(screen.contains("Esc cancel"), "{screen:?}");
        assert!(root.join("kept.txt").exists());
        assert!(!root.join("invented.txt").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn long_filesystem_plan_keeps_a_review_cursor_and_explicit_bounds() {
        let root = std::env::temp_dir().join(format!(
            "runyte-ui-long-fs-plan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let snapshot = crate::fs_plan::DirectorySnapshot::read(&root).unwrap();
        let desired = (0..40)
            .map(|row| {
                crate::fs_plan::DesiredEntry::create(
                    format!("entry-{row:02}"),
                    crate::fs_plan::EntryKind::File,
                )
            })
            .collect();
        let plan = crate::fs_plan::FsPlan::build(root.clone(), snapshot, desired).unwrap();
        let mut app = App::new(Config::default(), None).unwrap();
        app.fs_confirmation = Some(crate::app::FsConfirmation {
            buffer: 0,
            plan,
            selected: 0,
        });

        app.handle_key(crate::input::KeyStroke::new(
            crate::input::KeyCode::End,
            crate::input::Modifiers::NONE,
        ))
        .unwrap();
        let overlay = app
            .overlay_snapshots()
            .into_iter()
            .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::FilesystemConfirmation)
            .unwrap();
        assert_eq!(overlay.total_rows, 40);
        assert_eq!(overlay.scroll_anchor, Some(39));
        assert_eq!(app.fs_confirmation.as_ref().unwrap().selected, 39);
        let screen = rendered(&mut app, 70, 12);
        assert!(screen.contains("40/40"));
        assert!(screen.contains("entry-39"));
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn search_prompt_uses_search_prefix_without_command_palette() {
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new(Config::default(), None).unwrap();
        app.handle_key(crate::input::KeyStroke::new(
            crate::input::KeyCode::Char('/'),
            crate::input::Modifiers::NONE,
        ))
        .unwrap();
        app.handle_key(crate::input::KeyStroke::new(
            crate::input::KeyCode::Char('α'),
            crate::input::Modifiers::NONE,
        ))
        .unwrap();

        let hints = KeyHintState::default();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(rendered.contains("search (regex): α"));
        assert!(!rendered.contains("Commands"));
    }

    #[test]
    fn normal_snapshot_surface_renders_at_narrow_normal_and_wide_sizes() {
        for (width, height) in [(12, 8), (80, 24), (160, 40)] {
            let mut app = App::new(Config::default(), None).unwrap();
            app.buffers[0].apply(&Transaction::insert(0, "alpha\nbeta"));

            let screen = rendered(&mut app, width, height);

            assert!(screen.contains("alpha"), "{width}x{height}: {screen:?}");
            assert!(screen.contains("beta"), "{width}x{height}: {screen:?}");
        }
    }

    /// Renders once and returns the screen as text, so a test can assert what
    /// a person would see.
    fn rendered(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let hints = KeyHintState::default();
        terminal
            .draw(|frame| render_test_frame(frame, app, &hints))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    #[test]
    fn normal_view_preparation_is_idempotent_and_render_is_immutable() {
        let mut config = Config::default();
        config.editor.scroll_offset = 2;
        let mut app = App::new(config, None).unwrap();
        let content = (0..80)
            .map(|row| format!("line {row}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.buffers[0].apply(&Transaction::insert(0, &content));
        let offset = app.buffers[0].offset_of(Position::new(60, 0));
        let pane = app.panes.get_mut(&0).unwrap();
        pane.selection = Selection::point(offset);
        pane.scroll_col = 20;

        assert_preparation_is_idempotent_and_render_is_immutable(&mut app, 80, 24);

        assert_eq!(app.areas.len(), 1);
        assert!(app.active().scroll_row > 0);
        assert_eq!(app.active().scroll_col, 0);
        assert!(app.active().wrap_width > 1);
    }

    #[test]
    fn wrapped_split_view_preparation_is_idempotent_and_render_is_immutable() {
        let mut config = Config::default();
        config.editor.soft_wrap = true;
        config.editor.line_numbers = false;
        config.editor.scroll_offset = 1;
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(
            0,
            "alpha bravo charlie delta echo foxtrot\nsecond long wrapped line\nlast",
        ));
        for character in [' ', 'w', 'v'] {
            app.handle_key(crate::input::KeyStroke::new(
                crate::input::KeyCode::Char(character),
                crate::input::Modifiers::NONE,
            ))
            .unwrap();
        }
        let offset = app.buffers[0].offset_of(Position::new(1, 12));
        for pane in app.panes.values_mut() {
            pane.selection = Selection::point(offset);
            pane.scroll_row = usize::MAX;
            pane.scroll_wrap = usize::MAX;
            pane.scroll_col = 7;
        }

        assert_preparation_is_idempotent_and_render_is_immutable(&mut app, 40, 12);

        assert_eq!(app.areas.len(), 2);
        assert!(app.panes.values().all(|pane| pane.scroll_col == 0));
        assert!(app.panes.values().all(|pane| pane.wrap_width > 1));
    }

    #[test]
    fn tiny_pane_preparation_preserves_existing_viewport_values() {
        let mut app = App::new(Config::default(), None).unwrap();
        let pane = app.panes.get_mut(&0).unwrap();
        pane.scroll_row = 7;
        pane.scroll_wrap = 3;
        pane.scroll_col = 11;
        pane.wrap_width = 19;
        let before = ViewFingerprint::capture(&app);
        let geometry = FrameGeometry {
            screen: Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 4,
            },
            editor: Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
            status: Rect {
                x: 0,
                y: 2,
                width: 2,
                height: 1,
            },
            message: Rect {
                x: 0,
                y: 3,
                width: 2,
                height: 1,
            },
        };

        let prepared = app.prepare_view(geometry);
        let pane = prepared.pane(0).unwrap();

        assert!(!pane.drawable);
        assert_eq!(pane.area, geometry.editor);
        assert_eq!((pane.scroll_row, pane.scroll_wrap), (7, 3));
        assert_eq!((pane.scroll_col, pane.wrap_width), (11, 19));
        assert_eq!(app.active().scroll_row, 7);
        assert_eq!(app.active().scroll_wrap, 3);
        assert_eq!(app.active().scroll_col, 11);
        assert_eq!(app.active().wrap_width, 19);
        assert_ne!(ViewFingerprint::capture(&app), before);
    }

    fn rust_buffer(app: &mut App, text: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("runyte-ui-{}.rs", std::process::id()));
        app.buffers[0].path = Some(path.clone());
        app.buffers[0].kind = crate::buffer::BufferKind::File;
        app.buffers[0].apply(&Transaction::insert(0, text));
        path
    }

    #[test]
    fn diagnostics_render_a_sign_column_and_an_inline_message() {
        let mut app = App::new(Config::default(), None).unwrap();
        let path = rust_buffer(&mut app, "let x = 1;\nlet y = 2;\n");
        app.apply_lsp_event(crate::lsp::LspEvent::Diagnostics {
            language: "rust".to_owned(),
            path,
            diagnostics: vec![crate::lsp::Diagnostic::new(lsp_types::Diagnostic {
                range: crate::lsp::LspRange::new(
                    crate::lsp::LspPosition::new(0, 4),
                    crate::lsp::LspPosition::new(0, 5),
                ),
                severity: Some(lsp_types::DiagnosticSeverity::ERROR),
                message: "unused variable".to_owned(),
                ..Default::default()
            })],
        });

        let prepared = app.prepare_view(frame_geometry(TuiRect::new(0, 0, 100, 24)));
        let pane = prepared.pane(0).unwrap();
        assert!(pane.signs);
        assert_eq!(pane.line_digits, 1);
        assert_eq!(pane.gutter_width, 5);
        assert_eq!(pane.text_width, pane.body_width - pane.gutter_width);
        let snapshot = app.snapshot(&prepared);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("diagnostic row is visible");
        };
        assert_eq!(row.diagnostic_sign, Some(crate::lsp::Severity::Error));
        assert!(row.runs.iter().any(|run| {
            matches!(
                run.kind,
                TextRunKind::Text {
                    diagnostic: Some(crate::lsp::Severity::Error),
                    ..
                }
            )
        }));
        assert!(row.runs.iter().any(|run| {
            matches!(
                run.kind,
                TextRunKind::InlineDiagnostic(crate::lsp::Severity::Error)
            )
        }));

        let screen = rendered(&mut app, 100, 24);
        assert!(screen.contains("unused variable"), "no inline message");
        // The sign column shifts the text right by exactly one cell.
        assert!(screen.contains("E1 │ let x"), "no sign column");
        assert!(
            screen.contains(" 2 │ let y"),
            "clean rows keep a blank sign"
        );
    }

    #[test]
    fn long_inline_diagnostics_are_bounded_to_remaining_viewport_cells() {
        let mut app = App::new(Config::default(), None).unwrap();
        let path = rust_buffer(&mut app, "x\n");
        app.apply_lsp_event(crate::lsp::LspEvent::Diagnostics {
            language: "rust".to_owned(),
            path,
            diagnostics: vec![crate::lsp::Diagnostic::new(lsp_types::Diagnostic {
                range: crate::lsp::LspRange::new(
                    crate::lsp::LspPosition::new(0, 0),
                    crate::lsp::LspPosition::new(0, 1),
                ),
                severity: Some(lsp_types::DiagnosticSeverity::ERROR),
                message: "界".repeat(10_000),
                ..Default::default()
            })],
        });
        let prepared = app.prepare_view(frame_geometry(TuiRect::new(0, 0, 20, 8)));
        let text_width = prepared.pane(0).unwrap().text_width;
        let snapshot = app.snapshot(&prepared);
        let SnapshotRow::Text(row) = &snapshot.pane(0).unwrap().rows[0] else {
            panic!("diagnostic row is visible");
        };
        let cells = row
            .runs
            .iter()
            .flat_map(|run| run.text.chars())
            .map(|character| {
                unicode_width::UnicodeWidthChar::width(character)
                    .unwrap_or(0)
                    .max(1)
            })
            .sum::<usize>();
        let inline = row
            .runs
            .iter()
            .find(|run| matches!(run.kind, TextRunKind::InlineDiagnostic(_)))
            .unwrap();

        assert!(cells <= text_width);
        assert!(inline.text.len() < 100);
    }

    #[test]
    fn wrapped_eol_diagnostic_belongs_only_to_the_final_segment() {
        let mut config = Config::default();
        config.editor.soft_wrap = true;
        config.editor.line_numbers = false;
        let mut app = App::new(config, None).unwrap();
        let path = rust_buffer(&mut app, "abcdefgh");
        app.panes.get_mut(&0).unwrap().selection = Selection::point(8);
        app.apply_lsp_event(crate::lsp::LspEvent::Diagnostics {
            language: "rust".to_owned(),
            path,
            diagnostics: vec![crate::lsp::Diagnostic::new(lsp_types::Diagnostic {
                range: crate::lsp::LspRange::new(
                    crate::lsp::LspPosition::new(0, 0),
                    crate::lsp::LspPosition::new(0, 1),
                ),
                severity: Some(lsp_types::DiagnosticSeverity::ERROR),
                message: "at eof".to_owned(),
                ..Default::default()
            })],
        });
        let prepared = app.prepare_view(frame_geometry(TuiRect::new(0, 0, 8, 8)));
        assert_eq!(prepared.pane(0).unwrap().text_width, 5);
        let snapshot = app.snapshot(&prepared);
        let pane = snapshot.pane(0).unwrap();
        let has_inline = |row: &SnapshotRow| match row {
            SnapshotRow::Text(row) => row
                .runs
                .iter()
                .any(|run| matches!(run.kind, TextRunKind::InlineDiagnostic(_))),
            SnapshotRow::Placeholder | SnapshotRow::Padding | SnapshotRow::Filler => false,
        };

        assert!(!has_inline(&pane.rows[0]));
        assert!(has_inline(&pane.rows[1]));
    }

    #[test]
    fn language_server_popups_render_without_disturbing_the_text() {
        let mut app = App::new(Config::default(), None).unwrap();
        rust_buffer(&mut app, "fn main() {\n    todo!()\n}\n");

        app.hover = Some(crate::app::HoverState {
            lines: vec!["fn main()".to_owned(), "the entry point".to_owned()],
        });
        let screen = rendered(&mut app, 100, 24);
        assert!(screen.contains("the entry point"));
        assert!(screen.contains("fn main() {"), "the buffer is still drawn");

        app.hover = None;
        app.signature = Some(crate::app::SignatureState {
            signatures: vec![crate::lsp::SignatureLine {
                label: "fn write(bytes: &[u8]) -> Result<()>".to_owned(),
                documentation: String::new(),
                active_parameter: Some((9, 21)),
            }],
        });
        assert!(rendered(&mut app, 100, 24).contains("bytes: &[u8]"));
    }

    #[test]
    fn malformed_signature_parameter_ranges_render_without_emphasis() {
        let mut app = App::new(Config::default(), None).unwrap();
        rust_buffer(&mut app, "fn main() {}\n");

        for active_parameter in [Some((5, 3)), Some((3, 4)), Some((3, 99))] {
            app.signature = Some(crate::app::SignatureState {
                signatures: vec![crate::lsp::SignatureLine {
                    label: "fn(é)".to_owned(),
                    documentation: String::new(),
                    active_parameter,
                }],
            });
            assert!(rendered(&mut app, 100, 24).contains("fn(é)"));
        }
    }

    #[test]
    fn a_result_picker_renders_its_rows_and_filter() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.list = Some(crate::picker::ListPicker::new(
            "Document symbols",
            vec![
                crate::picker::PickerItem::new("alpha", "function", 0),
                crate::picker::PickerItem::new("beta", "struct", 1),
            ],
        ));
        let screen = rendered(&mut app, 100, 24);
        assert!(screen.contains("Document symbols"));
        assert!(screen.contains("alpha"));
        assert!(screen.contains("beta"));
    }

    #[test]
    fn standalone_reports_advertise_and_immediately_apply_scrolling() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.list = Some(
            crate::picker::ListPicker::new(
                "Service report",
                (0..20)
                    .map(|index| {
                        crate::picker::PickerItem::new(format!("report-row-{index:02}"), "", index)
                    })
                    .collect(),
            )
            .as_report(),
        );

        let initial = rendered(&mut app, 100, 12);
        assert!(initial.contains("↑/↓ scroll"), "{initial}");
        assert!(initial.contains("report-row-00"), "{initial}");

        app.handle_key(crate::input::KeyStroke::new(
            crate::input::KeyCode::Down,
            crate::input::Modifiers::NONE,
        ))
        .unwrap();
        let scrolled = rendered(&mut app, 100, 12);
        assert!(!scrolled.contains("report-row-00"), "{scrolled}");
        assert!(scrolled.contains("report-row-01"), "{scrolled}");
    }

    #[test]
    fn hover_omissions_follow_the_popup_inner_height() {
        let mut app = App::new(Config::default(), None).unwrap();
        rust_buffer(&mut app, "fn main() {}\n");
        app.hover = Some(crate::app::HoverState {
            lines: (0..7).map(|line| format!("hover-line-{line}")).collect(),
        });

        let screen = rendered(&mut app, 100, 10);

        assert!(screen.contains("hover-line-5"), "{screen}");
        assert!(!screen.contains("hover-line-6"), "{screen}");
        assert!(screen.contains("1 more"), "{screen}");
        assert!(screen.contains("Enter full view"), "{screen}");
    }

    #[test]
    fn numeric_setting_input_renders_as_a_bounded_popup() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.mode = Mode::Command;
        app.prompt_kind = PromptKind::SettingValue(crate::settings::SettingId::EditorHardWrapWidth);
        app.command = "80".to_owned();
        app.command_cursor = 2;

        let screen = rendered(&mut app, 100, 24);

        assert!(screen.contains("editor.hard_wrap_width"), "{screen}");
        assert!(screen.contains("integer 1–1000"), "{screen}");
        assert!(screen.contains("Enter save"), "{screen}");
    }

    #[test]
    fn fuzzy_file_picker_renders_ranked_paths_preview_and_narrow_fallback() {
        let mut app = App::new(Config::default(), None).unwrap();
        let root = std::path::PathBuf::from("/project");
        let mut picker = crate::file_picker::FilePicker::new(1, root.clone());
        picker.add_paths(vec![
            crate::file_picker::ScanEntry::file(root.join("src/ui/file_picker.rs")),
            crate::file_picker::ScanEntry::file(root.join("src/picker.rs")),
        ]);
        picker.finish(0, false);
        picker.insert_query_text("picker");
        picker.preview = Some(crate::file_picker::FilePreview::from_text(
            "preview heading\npreview body",
        ));
        app.picker = Some(picker);

        let wide = rendered(&mut app, 120, 30);
        assert!(wide.contains("src/picker.rs"));
        assert!(wide.contains("preview heading"));
        assert!(wide.contains("2/2"));

        let narrow = rendered(&mut app, 60, 20);
        assert!(narrow.contains("src/picker.rs"));
        assert!(!narrow.contains("preview heading"));

        let truncated = matched_path_line(
            "路径/界面.rs",
            &[],
            8,
            ratatui::style::Color::White,
            ratatui::style::Color::Red,
        );
        assert!(
            truncated
                .spans
                .iter()
                .map(|span| span.width())
                .sum::<usize>()
                <= 8,
            "path truncation must use terminal cells"
        );
        assert!(
            truncated
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
                .ends_with("界面.rs"),
            "left truncation should retain a wide-character basename"
        );
    }

    #[test]
    fn resource_finder_renders_buffer_preview_and_narrow_fallback() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.picker = Some(crate::file_picker::FilePicker::new(
            1,
            std::path::PathBuf::from("/project"),
        ));
        let mut finder = crate::finder::ResourceFinder::default();
        finder.mode = crate::finder::FinderMode::Resources;
        finder.replace_items(
            vec![
                crate::finder::ResourceItem::new(
                    "notes.txt",
                    "src/notes.txt",
                    crate::finder::ResourceTarget::Buffer(0),
                    crate::finder::ResourceKind::Buffer,
                    ["buffer".to_owned(), "notes.txt".to_owned()],
                )
                .with_preview("authoritative buffer preview"),
            ],
            "",
        );
        app.finder = Some(finder);

        let wide = rendered(&mut app, 120, 30);
        assert!(wide.contains("Ctrl-t preview"), "{wide}");
        assert!(wide.contains("authoritative buffer preview"), "{wide}");

        let narrow = rendered(&mut app, 60, 20);
        assert!(narrow.contains("notes.txt"), "{narrow}");
        assert!(!narrow.contains("authoritative buffer preview"), "{narrow}");
    }

    #[test]
    fn fuzzy_grep_picker_keeps_paths_in_the_list_and_content_in_the_preview() {
        let mut app = App::new(Config::default(), None).unwrap();
        let root = std::path::PathBuf::from("/project");
        let mut picker = crate::file_picker::FilePicker::grep(2, root.clone());
        picker.add_content(vec![crate::file_picker::FileHits {
            path: root.join("src/main.rs"),
            lines: vec![crate::file_picker::LineHit {
                row: 8,
                column: 4,
                text: "launch workspace scanner".to_owned(),
            }],
        }]);
        picker.finish(0, false);
        picker.insert_query_text("wscan");
        let preview_text = (0..14)
            .map(|row| {
                if row == 0 {
                    "file head sentinel".to_owned()
                } else if row == 8 {
                    "launch workspace scanner".to_owned()
                } else {
                    format!("context line {}", row + 1)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        picker.preview = Some(crate::file_picker::FilePreview::snippet_from_text(
            &preview_text,
            8,
            vec![7, 11, 18, 19, 20],
        ));
        app.picker = Some(picker);

        let screen = rendered(&mut app, 100, 24);
        assert!(screen.contains("Fuzzy grep"), "{screen}");
        assert!(screen.contains("src/main.rs:9"), "{screen}");
        assert!(
            screen.contains("›  9 │ launch workspace scanner"),
            "{screen}"
        );
        assert!(screen.contains("launch workspace scanner"), "{screen}");
        assert!(!screen.contains("file head sentinel"), "{screen}");
        assert_eq!(
            app.picker
                .as_ref()
                .unwrap()
                .selected_entry()
                .unwrap()
                .label(),
            "src/main.rs:9"
        );

        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        let hints = KeyHintState::default();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        assert!(
            terminal.backend().buffer().content.iter().any(|cell| {
                cell.symbol() == "w"
                    && cell.style().fg == Some(to_tui_color(app.theme.foreground))
                    && cell.style().bg == Some(to_tui_color(app.theme.fuzzy_match_secondary))
            }),
            "non-contiguous preview matches should use the secondary match background"
        );

        app.picker.as_mut().unwrap().preview =
            Some(crate::file_picker::FilePreview::snippet_from_text(
                &preview_text,
                8,
                vec![7, 8, 9, 10, 11],
            ));
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        assert!(
            terminal.backend().buffer().content.iter().any(|cell| {
                cell.symbol() == "w"
                    && cell.style().fg == Some(to_tui_color(app.theme.foreground))
                    && cell.style().bg == Some(to_tui_color(app.theme.fuzzy_match_primary))
            }),
            "a contiguous preview match should use the primary match background"
        );

        // A query of several words lands one run a term, so its emphasis is
        // never contiguous. Each term did match whole, so it is a direct match
        // and has to keep the primary colour rather than read as gapped.
        {
            let picker = app.picker.as_mut().unwrap();
            picker.query.clear();
            picker.query_cursor = 0;
            picker.insert_query_text("launch scanner");
            picker.preview = Some(crate::file_picker::FilePreview::snippet_from_text(
                &preview_text,
                8,
                vec![0, 1, 2, 3, 4, 5, 17, 18, 19, 20, 21, 22, 23],
            ));
        }
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        assert!(
            terminal.backend().buffer().content.iter().any(|cell| {
                cell.symbol() == "s"
                    && cell.style().fg == Some(to_tui_color(app.theme.foreground))
                    && cell.style().bg == Some(to_tui_color(app.theme.fuzzy_match_primary))
            }),
            "each term of a several-word query matched whole, so the match is direct"
        );
    }

    #[test]
    fn commit_picker_renders_list_and_message_with_exact_and_fuzzy_selections() {
        let mut app = App::new(Config::default(), None).unwrap();
        let item = crate::picker::PickerItem::searchable(
            "abcdef123456 Refresh workspace Git state with a deliberately long title",
            "",
            "Refresh workspace Git state Ada 2026-08-16 abcdef123456",
            0,
        )
        .with_preview(
            "Ada Lovelace · 2026-08-16\n\nRefresh workspace Git state\nEntire body sentinel",
        );
        app.list = Some(
            crate::picker::ListPicker::fuzzy("Git commits", vec![item]).with_preview("Commit"),
        );
        for character in "workspace Git".chars() {
            app.list.as_mut().unwrap().push_filter(character);
        }

        let screen = rendered(&mut app, 100, 24);
        assert!(screen.contains("abcdef123456 Refresh"), "{screen}");
        assert!(screen.contains("Ada Lovelace · 2026-08-16"), "{screen}");
        assert!(screen.contains("Entire body sentinel"), "{screen}");

        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        let hints = KeyHintState::default();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        assert!(terminal.backend().buffer().content.iter().any(|cell| {
            cell.symbol() == "w"
                && cell.style().bg == Some(to_tui_color(app.theme.fuzzy_match_primary))
        }));

        app.list.as_mut().unwrap().clear_filter();
        for character in "wgs".chars() {
            app.list.as_mut().unwrap().push_filter(character);
        }
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        assert!(terminal.backend().buffer().content.iter().any(|cell| {
            cell.symbol() == "w"
                && cell.style().bg == Some(to_tui_color(app.theme.fuzzy_match_secondary))
        }));
    }

    #[test]
    fn buffer_picker_renders_its_contextual_action_menu() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.list = Some(
            crate::picker::ListPicker::new(
                "Buffers",
                vec![crate::picker::PickerItem::new("notes.txt", "modified", 0)],
            )
            .as_manager("open", "Tab", "actions"),
        );
        app.buffer_action_menu = Some(crate::app::BufferActionMenu {
            buffer: 0,
            actions: vec![
                crate::app::BufferAction::Save,
                crate::app::BufferAction::Discard,
            ],
            selected: 0,
        });

        let screen = rendered(&mut app, 100, 24);
        assert!(screen.contains("Save"));
        assert!(screen.contains("Discard changes"));
    }

    #[test]
    fn popups_survive_a_terminal_too_small_to_hold_them() {
        let mut app = App::new(Config::default(), None).unwrap();
        rust_buffer(&mut app, "fn main() {}\n");
        app.hover = Some(crate::app::HoverState {
            lines: (0..40).map(|line| format!("line {line}")).collect(),
        });
        // Must not panic at any size, including ones with no room at all.
        for (width, height) in [(100, 24), (20, 6), (8, 4), (3, 3), (1, 1)] {
            let _ = rendered(&mut app, width, height);
        }
    }

    #[test]
    fn explicit_view_alignment_survives_render_scroll_adjustment() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new(Config::default(), None).unwrap();
        let content = (0..100)
            .map(|row| format!("line {row}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.buffers[0].apply(&Transaction::insert(0, &content));
        let offset = app.buffers[0].offset_of(Position::new(50, 0));
        app.panes.get_mut(&0).unwrap().selection = Selection::point(offset);
        app.panes.get_mut(&0).unwrap().scroll_row = 50;
        app.panes.get_mut(&0).unwrap().preserve_scroll = true;

        let hints = KeyHintState::default();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();

        assert_eq!(app.active().scroll_row, 50);
    }

    #[test]
    fn soft_wrap_renders_continuation_rows_without_horizontal_scrolling() {
        let mut config = Config::default();
        config.editor.soft_wrap = true;
        config.editor.line_numbers = false;
        config.editor.scroll_offset = 0;
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "abcdefghijklmnop\nlast"));

        let screen = rendered(&mut app, 12, 8);
        assert!(screen.contains("abcdefghij"), "{screen:?}");
        assert!(screen.contains("klmnop"), "{screen:?}");
        assert_eq!(app.active().scroll_col, 0);
        assert_eq!(app.active().wrap_width, 10);
    }

    #[test]
    fn soft_wrap_marks_continuation_rows_in_the_line_number_gutter() {
        let mut config = Config::default();
        config.editor.soft_wrap = true;
        config.editor.line_numbers = true;
        config.editor.scroll_offset = 0;
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "abcdefghijklmnop\nlast"));

        let screen = rendered(&mut app, 16, 8);
        assert!(screen.contains("1 │ abcdefghij"), "{screen:?}");
        assert!(screen.contains(" ↪│ klmnop"), "{screen:?}");
        assert!(!screen.contains("↪abcdefghijk"), "{screen:?}");
    }

    #[test]
    fn a_command_prompt_dims_the_text_in_every_pane() {
        let mut config = Config::default();
        config.editor.line_numbers = false;
        config.theme = Some("light".into());
        let theme = config.resolve_theme("light").unwrap();
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "zzzz"));
        // A vertical split shows the same buffer twice, side by side, so the
        // same glyph has to be found dimmed in the focused and the unfocused
        // pane both.
        app.handle_key(crate::input::KeyStroke::new(
            crate::input::KeyCode::Char('w'),
            crate::input::Modifiers::CONTROL,
        ))
        .unwrap();
        app.handle_key(crate::input::KeyStroke::char('v')).unwrap();

        let hints = KeyHintState::default();
        // Every `z` except the one under the caret, which paints its glyph in
        // the background and is asserted separately below.
        let text = |app: &mut App| {
            let mut terminal = Terminal::new(TestBackend::new(40, 14)).unwrap();
            terminal
                .draw(|frame| render_test_frame(frame, app, &hints))
                .unwrap();
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .filter(|cell| {
                    cell.symbol() == "z" && cell.style().fg != Some(to_tui_color(theme.background))
                })
                .map(|cell| cell.style().fg)
                .collect::<Vec<_>>()
        };

        let before = text(&mut app);
        assert!(
            before.len() >= 2,
            "both panes should show the buffer: {before:?}"
        );
        assert!(
            before
                .iter()
                .all(|color| *color == Some(to_tui_color(theme.foreground))),
            "{before:?}"
        );

        // Typed far enough to narrow the palette: an unfiltered one is as tall
        // as the panes it floats over and would hide the text under test.
        for character in ":quit".chars() {
            app.handle_key(crate::input::KeyStroke::char(character))
                .unwrap();
        }
        let dimmed = text(&mut app);
        assert_eq!(dimmed.len(), before.len());
        assert!(
            dimmed
                .iter()
                .all(|color| *color == Some(to_tui_color(theme.jump_text_muted))),
            "every pane's text should use the dimming role: {dimmed:?}"
        );

        app.handle_key(crate::input::KeyStroke::new(
            crate::input::KeyCode::Escape,
            crate::input::Modifiers::NONE,
        ))
        .unwrap();
        assert_eq!(text(&mut app), before);
    }

    #[test]
    fn the_command_prompt_and_its_caret_are_not_dimmed() {
        let mut config = Config::default();
        config.editor.line_numbers = false;
        config.theme = Some("light".into());
        let theme = config.resolve_theme("light").unwrap();
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "zzzz"));
        for character in ":quit".chars() {
            app.handle_key(crate::input::KeyStroke::char(character))
                .unwrap();
        }

        let hints = KeyHintState::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 14)).unwrap();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        // The caret is chrome rather than document text, so it keeps the
        // colour that names the mode instead of joining the dimmed run.
        assert!(
            buffer.content.iter().any(|cell| cell.symbol() == "z"
                && cell.style().bg == Some(to_tui_color(theme.cursor_command))),
            "the caret should keep the Command colour"
        );
        // The typed command is drawn below the panes and is never dimmed.
        let (column, row) = find_text(&buffer, ":quit").expect("the prompt is drawn");
        assert_eq!(
            buffer[(column, row)].style().fg,
            Some(to_tui_color(theme.foreground))
        );
    }

    #[test]
    fn jump_labels_paint_over_the_words_they_name() {
        let mut config = Config::default();
        config.editor.line_numbers = false;
        // Named rather than left to the default, so the colours asserted below
        // are the ones the app actually starts in.
        config.theme = Some("light".into());
        let theme = config.resolve_theme("light").unwrap();
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "alpha beta"));

        // The first frame records the text width used by `goto-word`.
        let _ = rendered(&mut app, 20, 6);

        for character in "gw".chars() {
            app.handle_key(crate::input::KeyStroke::new(
                crate::input::KeyCode::Char(character),
                crate::input::Modifiers::NONE,
            ))
            .unwrap();
        }

        let mut terminal = Terminal::new(TestBackend::new(20, 6)).unwrap();
        let hints = KeyHintState::default();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        let cells = terminal.backend().buffer().content.clone();

        // Nearby labels occupy one cell, so the rest of the line stays exactly
        // where it was.
        let screen: String = cells
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(screen.contains("alpha seta"), "{screen:?}");

        let colored = |symbol: &str, color| {
            cells
                .iter()
                .any(|cell| cell.symbol() == symbol && cell.style().fg == Some(to_tui_color(color)))
        };
        assert!(
            colored("a", theme.jump_label_immediate),
            "first immediate label"
        );
        assert!(
            colored("s", theme.jump_label_immediate),
            "second immediate label"
        );
        assert!(
            colored("l", theme.jump_text_muted),
            "ordinary text uses the dedicated jump-dimming role"
        );
        assert_ne!(theme.jump_text_muted, theme.muted);
    }

    #[test]
    fn distant_jump_labels_use_two_neon_cyans_then_narrow_to_one_red_key() {
        let mut config = Config::default();
        config.editor.line_numbers = false;
        config.theme = Some("base16".into());
        let theme = config.resolve_theme("base16").unwrap();
        let mut app = App::new(config, None).unwrap();
        let line = std::iter::repeat_n("aa", 27).collect::<Vec<_>>().join(" ");
        app.buffers[0].apply(&Transaction::insert(0, &line));

        let _ = rendered(&mut app, 120, 6);
        for character in "gw".chars() {
            app.handle_key(crate::input::KeyStroke::char(character))
                .unwrap();
        }

        let mut terminal = Terminal::new(TestBackend::new(120, 6)).unwrap();
        let hints = KeyHintState::default();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        let cells = terminal.backend().buffer().content.clone();
        let colored = |symbol: &str, color| {
            cells
                .iter()
                .any(|cell| cell.symbol() == symbol && cell.style().fg == Some(to_tui_color(color)))
        };
        assert!(colored("m", theme.jump_label_primary), "two-key prefix");
        assert!(colored("a", theme.jump_label_secondary), "two-key suffix");

        app.handle_key(crate::input::KeyStroke::char('m')).unwrap();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        let cells = terminal.backend().buffer().content.clone();
        assert!(cells.iter().any(|cell| {
            cell.symbol() == "a"
                && cell.style().fg == Some(to_tui_color(theme.jump_label_immediate))
        }));
        assert!(cells.iter().any(|cell| {
            cell.symbol() == "s"
                && cell.style().fg == Some(to_tui_color(theme.jump_label_immediate))
        }));
        assert!(!cells.iter().any(|cell| {
            matches!(
                cell.style().fg,
                Some(color)
                    if color == to_tui_color(theme.jump_label_primary)
                        || color == to_tui_color(theme.jump_label_secondary)
            )
        }));
    }

    #[test]
    fn jump_labels_stop_at_the_bottom_of_a_soft_wrapped_pane() {
        let mut config = Config::default();
        config.editor.soft_wrap = true;
        config.editor.line_numbers = false;
        config.editor.scroll_offset = 0;
        let mut app = App::new(config, None).unwrap();
        // One logical line in a pane three rows tall. Word-aware wrapping puts
        // one of its first three words at the start of each visible segment.
        app.buffers[0].apply(&Transaction::insert(
            0,
            "alpha bravo charlie delta echo foxtrot",
        ));

        // Render first: the pane learns the width it wraps at from the frame,
        // and that width is what decides which words are on screen.
        let mut terminal = Terminal::new(TestBackend::new(12, 7)).unwrap();
        let hints = KeyHintState::default();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        assert_eq!(app.active().wrap_width, 10);

        for character in "gw".chars() {
            app.handle_key(crate::input::KeyStroke::new(
                crate::input::KeyCode::Char(character),
                crate::input::Modifiers::NONE,
            ))
            .unwrap();
        }

        // Only the first three words are visible; later words must not hold a
        // label a person cannot see or reach.
        let labels = app.jump.as_ref().unwrap();
        assert_eq!(labels.len(), 3);
        assert_eq!(labels.label_at(31), None);
    }

    #[test]
    fn jump_labels_wrap_at_the_width_the_gutter_leaves() {
        let mut config = Config::default();
        config.editor.soft_wrap = true;
        config.editor.line_numbers = true;
        config.editor.scroll_offset = 0;
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(
            0,
            "alpha bravo charlie delta echo foxtrot",
        ));

        let mut terminal = Terminal::new(TestBackend::new(12, 7)).unwrap();
        let hints = KeyHintState::default();
        terminal
            .draw(|frame| render_test_frame(frame, &mut app, &hints))
            .unwrap();
        // The line-number column and its text margin cost four of the ten
        // cells inside the pane.
        assert_eq!(app.active().wrap_width, 6);

        for character in "gw".chars() {
            app.handle_key(crate::input::KeyStroke::new(
                crate::input::KeyCode::Char(character),
                crate::input::Modifiers::NONE,
            ))
            .unwrap();
        }

        // Word-aware wrapping every six columns puts alpha, bravo, and
        // charlie at the starts of the three visible segments. Measuring the
        // pane without its gutter would wrap at ten and expose different
        // words below the fold.
        let labels = app.jump.as_ref().unwrap();
        assert_eq!(labels.len(), 3);
        assert_eq!(labels.label_at(0), Some(('a', LabelPart::Immediate)));
        assert_eq!(labels.label_at(6), Some(('s', LabelPart::Immediate)));
        assert_eq!(labels.label_at(12), Some(('d', LabelPart::Immediate)));
        assert_eq!(labels.label_at(13), None);
    }
}
