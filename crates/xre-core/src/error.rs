//! The crate-local error type. Library code never panics (see the no-panic
//! policy in `CLAUDE.md`); fallible operations return [`Result`].

use crate::math::UVec2;

/// Errors produced by `xre-core`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum CoreError {
    /// A coordinate fell outside the bounds of a buffer.
    #[error("point ({}, {}) is outside a buffer of size {}x{}", point.x, point.y, size.x, size.y)]
    OutOfBounds {
        /// The offending coordinate.
        point: UVec2,
        /// The buffer's size.
        size: UVec2,
    },
}

/// A `Result` specialised to [`CoreError`].
pub type Result<T> = core::result::Result<T, CoreError>;
