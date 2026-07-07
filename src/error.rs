use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("git command failed: {0}")]
    GitCommand(#[from] GitError),
    #[error("failed to read changelist store at {path}: {source}")]
    StoreRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write changelist store at {path}: {source}")]
    StoreWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse changelist store: {0}")]
    StoreParse(#[from] serde_json::Error),
    #[error("shelf entry not found (already unshelved or deleted?)")]
    ShelfNotFound,
    #[error("not inside a git repository (or git is not installed)")]
    NotARepo,
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("failed to spawn git: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("git exited with {status}: {stderr}")]
    NonZeroExit { status: i32, stderr: String },
    #[error("failed to parse git output: {0}")]
    InvalidOutput(String),
}

pub type Result<T> = std::result::Result<T, AppError>;
