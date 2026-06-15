//! The terminal backend's error type.

/// Errors produced by `xre-term`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TermError {
    /// An underlying terminal I/O operation failed.
    #[error("terminal I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A `Result` specialised to [`TermError`].
pub type Result<T> = std::result::Result<T, TermError>;
