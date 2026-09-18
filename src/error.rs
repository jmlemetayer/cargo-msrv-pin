//! The app's error type.

use crate::command::CommandFailure;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Command(#[from] CommandFailure),

    #[error("{0}: command not found")]
    CommandNotFound(String),

    #[error("{action}")]
    Io {
        action: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid version '{version}'")]
    InvalidVersion {
        version: String,
        #[source]
        source: semver::Error,
    },

    #[error("failed to parse cargo metadata")]
    Metadata(#[from] serde_json::Error),

    #[error("{action} for '{crate_name}'")]
    CratesIo {
        action: &'static str,
        crate_name: String,
        #[source]
        source: reqwest::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
