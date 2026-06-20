mod app;
mod cli;
mod error;
mod git;
mod model;
mod ui;

use std::time::Duration;

use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};

use app::App;
use error::AppError;

fn main() -> anyhow::Result<()> {
    let args = cli::Args::parse();

    let repo_root = match args.repo {
        Some(path) => path,
        None => {
            let cwd = std::env::current_dir()?;
            git::discover_repo_root(&cwd).ok_or(AppError::NotARepo)?
        }
    };

    let mut app = App::new(repo_root)?;

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();

    result.map_err(Into::into)
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    while !app.should_quit {
        app.poll_push();
        terminal.draw(|frame| ui::draw(frame, app))?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Some(action) = ui::keymap::map_key(app, key.code, key.modifiers) {
                        app.dispatch(action)?;
                    }
                }
            }
        }
    }
    Ok(())
}
