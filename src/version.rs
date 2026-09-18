//! Toolchain version string parsing.
//!
//! Parses a possibly-abbreviated version string (e.g. "1.75") into a full
//! semver `Version`.

use crate::error::{Error, Result};
use semver::Version;

/// Parse a possibly-abbreviated version string (e.g. "1.75" or "1.75.0") into
/// a full semver `Version`, padding missing minor/patch components with 0.
pub fn parse_version(s: &str) -> Result<Version> {
    let padded = match s.matches('.').count() {
        0 => format!("{s}.0.0"),
        1 => format!("{s}.0"),
        _ => s.to_owned(),
    };
    Version::parse(&padded).map_err(|source| Error::InvalidVersion {
        version: s.to_owned(),
        source,
    })
}

/// The target toolchain, kept in both forms it's needed in: the exact
/// string cargo/rustup expect after `+` (a `Version` wouldn't necessarily
/// round-trip to it, e.g. "1.75" pads to "1.75.0"), and the parsed
/// `Version` used for MSRV comparisons.
pub struct Toolchain {
    pub raw: String,
    pub version: Version,
}

impl Toolchain {
    pub fn parse(raw: &str) -> Result<Self> {
        let version = parse_version(raw)?;
        Ok(Self {
            raw: raw.to_owned(),
            version,
        })
    }

    /// The `+<toolchain>` form `cargo`/`rustup` expect as an argument.
    pub fn arg(&self) -> String {
        format!("+{}", self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_major_only() {
        assert_eq!(parse_version("1").unwrap(), Version::new(1, 0, 0));
    }

    #[test]
    fn pads_major_minor() {
        assert_eq!(parse_version("1.75").unwrap(), Version::new(1, 75, 0));
    }

    #[test]
    fn keeps_full_version() {
        assert_eq!(parse_version("1.75.0").unwrap(), Version::new(1, 75, 0));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_version("not-a-version").is_err());
    }

    #[test]
    fn orders_as_expected() {
        assert!(parse_version("1.75").unwrap() < parse_version("1.75.1").unwrap());
        // Semver compares components numerically, not lexically: minor 8 < 75.
        assert!(parse_version("1.8").unwrap() < parse_version("1.75").unwrap());
    }
}
