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
use error::{AppError, GitError};

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
                        if app.wants_interactive_push {
                            app.wants_interactive_push = false;
                            run_interactive_push(terminal, app);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Suspends the TUI (cooked mode, main screen) and runs `git push` with
/// the real terminal, so git and ssh can prompt for credentials, key
/// passphrases and host-key confirmation natively — those prompts read
/// and write `/dev/tty` directly and would otherwise scribble over the
/// raw-mode UI. Pauses for Enter before re-entering the TUI so whatever
/// git/ssh printed stays readable.
fn run_interactive_push(terminal: &mut ratatui::DefaultTerminal, app: &mut App) {
    ratatui::restore();
    println!("Running `git push` — answer any credential/host-key prompts below.\n");
    let message = match git::index_ops::push_interactive(&app.repo_root) {
        Ok(()) => "pushed".to_string(),
        Err(GitError::NonZeroExit { status, .. }) => format!("push failed (exit {status})"),
        Err(e) => format!("push failed: {e}"),
    };
    println!("\n{message} — press Enter to return to gitcl");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    *terminal = ratatui::init();
    app.status_message = Some(message);
    if let Err(e) = app.refresh() {
        app.status_message = Some(format!("refresh failed: {e}"));
    }
}
