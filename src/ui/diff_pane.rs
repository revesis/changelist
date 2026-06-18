use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{App, DiffMode, Pane};

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_pane == Pane::Diff;
    let title = match (app.selected_file_path(), app.diff_mode) {
        (Some(path), DiffMode::WorkTree) => format!("Diff (working tree): {path}"),
        (Some(path), DiffMode::Staged) => format!("Diff (staged): {path}"),
        (None, _) => "Diff".to_string(),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });

    let body: Vec<Line> = match app.selected_file_diff() {
        Some(Ok(text)) if !text.is_empty() => text.lines().map(colored_diff_line).collect(),
        Some(Ok(_)) => vec![Line::from("(no diff in this mode)")],
        Some(Err(e)) => vec![Line::from(Span::styled(
            format!("diff failed: {e}"),
            Style::default().fg(Color::Red),
        ))],
        None => vec![Line::from("(no file selected)")],
    };

    frame.render_widget(
        Paragraph::new(body)
            .block(block)
            .scroll((app.diff_scroll, app.diff_hscroll)),
        area,
    );
}

fn colored_diff_line(line: &str) -> Line<'static> {
    let owned = line.to_string();
    let style = if owned.starts_with('+') && !owned.starts_with("+++") {
        Style::default().fg(Color::Green)
    } else if owned.starts_with('-') && !owned.starts_with("---") {
        Style::default().fg(Color::Red)
    } else if owned.starts_with("@@") {
        Style::default().fg(Color::Cyan)
    } else if owned.starts_with("diff --git") || owned.starts_with("index ") {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    Line::from(Span::styled(owned, style))
}
