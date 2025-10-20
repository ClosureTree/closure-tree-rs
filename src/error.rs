use thiserror::Error;

/// Errors returned by the closure-tree helper APIs.
#[derive(Debug, Error)]
pub enum ClosureTreeError {
    #[error("closure-tree currently supports PostgreSQL connections only")]
    UnsupportedBackend,

    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("path cannot be empty")]
    EmptyPath,

    #[error("closure-tree invariant violation: {0}")]
    Invariant(String),
}

impl ClosureTreeError {
    pub fn invariant(detail: impl Into<String>) -> Self {
        Self::Invariant(detail.into())
    }
}
