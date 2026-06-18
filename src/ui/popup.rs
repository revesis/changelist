use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::app::{App, Popup};

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    area
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let Some(popup) = &app.popup else { return };

    match popup {
        Popup::NewChangelist { buffer } => {
            draw_text_input(frame, area, "New changelist name", buffer);
        }
        Popup::Rename { buffer, .. } => {
            draw_text_input(frame, area, "Rename changelist", buffer);
        }
        Popup::ConfirmDelete { id } => {
            let name = app
                .store
                .changelist_by_id(id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            let msg = format!(
                "Delete changelist \"{name}\"? Files move to Default.\n[Enter] confirm  [Esc] cancel"
            );
            draw_message(frame, area, "Confirm delete", &msg);
        }
        Popup::MoveFile { paths, selected } => {
            draw_picker(frame, app, area, paths, *selected);
        }
        Popup::CommitMessage { id, buffer } => {
            let name = app
                .store
                .changelist_by_id(id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            draw_text_input(frame, area, &format!("Commit message for \"{name}\""), buffer);
        }
    }
}

fn draw_text_input(frame: &mut Frame, area: Rect, title: &str, buffer: &str) {
    let popup_area = centered(area, 50, 3);
    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let text = format!("{buffer}_");
    frame.render_widget(Paragraph::new(text).block(block), popup_area);
}

fn draw_message(frame: &mut Frame, area: Rect, title: &str, message: &str) {
    let popup_area = centered(area, 60, 5);
    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    frame.render_widget(Paragraph::new(message).block(block), popup_area);
}

fn draw_picker(frame: &mut Frame, app: &App, area: Rect, paths: &[String], selected: usize) {
    let popup_area = centered(area, 50, (app.store.changelists.len() as u16 + 2).min(12));
    frame.render_widget(Clear, popup_area);
    let items: Vec<ListItem> = app
        .store
        .changelists
        .iter()
        .enumerate()
        .map(|(idx, cl)| {
            let style = if idx == selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default()
            };
            ListItem::new(cl.name.clone()).style(style)
        })
        .collect();
    let title = match paths {
        [single] => format!("Move \"{single}\" to..."),
        many => format!("Move {} files to...", many.len()),
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    frame.render_widget(List::new(items).block(block), popup_area);
}
