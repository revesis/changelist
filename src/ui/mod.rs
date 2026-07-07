pub mod diff_pane;
pub mod keymap;
pub mod popup;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::app::{App, Pane, TreeRow};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[0]);

    draw_tree_pane(frame, app, panes[0]);
    diff_pane::draw(frame, app, panes[1]);
    draw_status_bar(frame, app, chunks[1]);
    popup::draw(frame, app, area);
    if app.show_help {
        draw_help(frame, area);
    }
}

fn draw_help(frame: &mut Frame, area: ratatui::layout::Rect) {
    use ratatui::layout::{Constraint, Flex};
    use ratatui::widgets::Clear;

    let lines = [
        "Tab / Shift+Tab   cycle pane focus (tree/diff)",
        "j/k, Up/Down      move selection, or scroll diff vertically when focused",
        "h/l, Left/Right   scroll the focused pane horizontally (long paths/lines)",
        "Enter             fold/unfold changelist",
        "Space             stage/unstage selected (or visual-range) files",
        "Shift+V           toggle visual mode on a file row (batch select)",
        "Esc               exit visual mode",
        "v                 toggle working-tree/staged diff",
        "n                 new changelist",
        "r                 rename selected changelist (cursor on header)",
        "d                 delete selected changelist (cursor on header)",
        "a                 set selected changelist active (cursor on header)",
        "m                 move selected (or visual-range) files (cursor on file)",
        "c                 commit selected changelist (cursor on header)",
        "Shift+S           shelve selected changelist (cursor on header)",
        "Shift+U           unshelve / manage shelved changes",
        "Shift+P           push current branch (if it fails needing",
        "                  input, press again to push interactively)",
        "Ctrl+R / F5       manual refresh",
        "?                 toggle this help",
        "q / Ctrl+C        quit",
        "",
        "(press any key to close)",
    ];
    let height = lines.len() as u16 + 2;
    let [help_area] = Layout::horizontal([Constraint::Length(64)])
        .flex(Flex::Center)
        .areas(area);
    let [help_area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(help_area);

    frame.render_widget(Clear, help_area);
    let block = Block::default()
        .title("Help")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    frame.render_widget(Paragraph::new(lines.join("\n")).block(block), help_area);
}

/// Drops the first `offset` characters of `text`, for manual horizontal
/// scrolling of `List` rows (which, unlike `Paragraph`, has no built-in
/// scroll offset).
fn hscrolled(text: &str, offset: u16) -> String {
    if offset == 0 {
        return text.to_string();
    }
    text.chars().skip(offset as usize).collect()
}

fn highlight_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Gray)
    }
}

fn draw_tree_pane(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let focused = app.focused_pane == Pane::Tree;
    let visual_rows = app.visual_file_rows().unwrap_or_default();

    let items: Vec<ListItem> = app
        .tree_rows
        .iter()
        .enumerate()
        .map(|(row_idx, row)| match row {
            TreeRow::Header(cl_idx) => {
                let cl = &app.store.changelists[*cl_idx];
                let icon = if app.collapsed.contains(cl_idx) { "▶" } else { "▼" };
                let marker = if cl.id == app.store.active_changelist { "* " } else { "  " };
                let count = app.store.files_in(&cl.id).len();
                let text = hscrolled(
                    &format!("{icon} {marker}{} ({count})", cl.name),
                    app.tree_hscroll,
                );
                let style = if row_idx == app.tree_cursor {
                    highlight_style(focused)
                } else {
                    Style::default().add_modifier(Modifier::BOLD)
                };
                ListItem::new(Line::from(Span::styled(text, style)))
            }
            TreeRow::File(cl_idx, file_idx) => {
                let cl = &app.store.changelists[*cl_idx];
                let files = app.store.files_in(&cl.id);
                let path = files.get(*file_idx).copied().unwrap_or("");
                let badge = status_badge(app, path);
                let file_name = std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string());
                let text = hscrolled(
                    &format!("  {badge} {file_name}  {path}"),
                    app.tree_hscroll,
                );
                let style = if row_idx == app.tree_cursor {
                    highlight_style(focused)
                } else if visual_rows.contains(&row_idx) {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(text, style)))
            }
        })
        .collect();

    let title = if app.visual_anchor.is_some() {
        "Changes [VISUAL]"
    } else {
        "Changes"
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });

    let mut state = ListState::default().with_selected(Some(app.tree_cursor));
    frame.render_stateful_widget(List::new(items).block(block), area, &mut state);
}

fn status_badge(app: &App, path: &str) -> String {
    use crate::git::status::ChangeKind;
    let Some(entry) = app.status_entry_for(path) else {
        return "[ ]".to_string();
    };
    if entry.untracked {
        return "[?]".to_string();
    }
    let letter = |k: ChangeKind| match k {
        ChangeKind::Modified => "M",
        ChangeKind::Added => "A",
        ChangeKind::Deleted => "D",
        ChangeKind::Renamed => "R",
        ChangeKind::Copied => "C",
        ChangeKind::TypeChanged => "T",
        ChangeKind::Unmerged => "U",
        ChangeKind::Unmodified => ".",
    };
    format!("[{}{}]", letter(entry.staged), letter(entry.worktree))
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let text = app.status_message.clone().unwrap_or_else(|| {
        "Tab:pane j/k:move h/l:hscroll Space:stage V:visual v:diff-mode n:new r:rename d:delete a:active m:move c:commit S:shelve U:unshelve P:push ?:help q:quit"
            .to_string()
    });
    frame.render_widget(Paragraph::new(text), area);
}
