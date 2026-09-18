//! Cargo.lock format recovery.
//!
//! Recovers from a Cargo.lock written in a lock-file format newer than the
//! target toolchain's cargo can parse, by deleting and regenerating it.

use crate::error::{Error, Result};
use crate::metadata::cargo_metadata;
use std::path::Path;

const LOCK_FILE: &str = "Cargo.lock";
const VERSION_ERROR_MARKER: &str = "lock file version";

/// Ensure `Cargo.lock` is in a format `toolchain` can parse.
///
/// A lock file written by a newer cargo can carry a lock-file version an
/// older toolchain does not understand ("lock file version `4` was found,
/// but this version of Cargo does not understand this lock file"), which
/// fails before any dependency is even looked at. When that happens, delete
/// it and let `toolchain` regenerate it: the same command still fails
/// afterwards, since no dependency has been downgraded yet, but the lock
/// file it leaves behind is one that toolchain can read, so the normal
/// downgrade loop can proceed. Any other failure here is left as-is: it is
/// the expected pre-downgrade state and the caller's own build/check step
/// will report it.
pub fn ensure_readable_by(toolchain: &str) -> Result<()> {
    let Err(e) = cargo_metadata(Some(toolchain)) else {
        return Ok(());
    };
    let is_version_mismatch =
        matches!(&e, Error::Command(f) if f.stderr_contains(VERSION_ERROR_MARKER));
    if !is_version_mismatch {
        return Ok(());
    }

    log::warn!("Cargo.lock format is not understood by toolchain {toolchain}, regenerating it");
    if Path::new(LOCK_FILE).exists() {
        std::fs::remove_file(LOCK_FILE).map_err(|source| Error::Io {
            action: "removing incompatible Cargo.lock",
            source,
        })?;
    }
    if let Err(e) = cargo_metadata(Some(toolchain)) {
        log::debug!("expected failure while regenerating Cargo.lock: {e}");
    }
    Ok(())
}
