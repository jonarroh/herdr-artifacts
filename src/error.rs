//! Crate-wide error type. Every variant maps to a stable `code` that is sent
//! back to agents over IPC and printed by the CLI.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// The artifact, patch or request failed validation.
    Invalid(String),
    /// The referenced artifact does not exist.
    NotFound(String),
    /// A file or payload exceeds a configured limit.
    TooLarge(String),
    /// The IPC token was missing or wrong.
    Unauthorized,
    /// Filesystem or socket failure.
    Io(String),
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Error::Invalid(_) => "invalid",
            Error::NotFound(_) => "not_found",
            Error::TooLarge(_) => "too_large",
            Error::Unauthorized => "unauthorized",
            Error::Io(_) => "io",
        }
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(m) => write!(f, "invalid artifact: {m}"),
            Error::NotFound(id) => write!(f, "artifact not found: {id}"),
            Error::TooLarge(m) => write!(f, "too large: {m}"),
            Error::Unauthorized => write!(f, "unauthorized: bad or missing IPC token"),
            Error::Io(m) => write!(f, "io error: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Invalid(format!("json: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
