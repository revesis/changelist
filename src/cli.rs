use std::path::PathBuf;

#[derive(Debug, clap::Parser)]
#[command(name = "gitcl", about = "IDEA-style changelist TUI for git")]
pub struct Args {
    /// Path to the git repository (defaults to discovering from cwd)
    #[arg(long)]
    pub repo: Option<PathBuf>,
}
