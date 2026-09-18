//! crates.io version lookups.
//!
//! Looks up a crate's published versions and picks the highest one whose
//! declared MSRV does not exceed the target toolchain.

use crate::error::{Error, Result};
use crate::version::parse_version;
use semver::Version;
use serde::Deserialize;

/// crates.io asks API clients to identify themselves and provide a way to
/// be reached (<https://crates.io/data-access>). Built from Cargo.toml's own
/// `name` and `repository` rather than duplicated as a literal, so it can't
/// drift from the package's actual identity.
const USER_AGENT: &str = concat!(
    env!("CARGO_PKG_NAME"),
    " (",
    env!("CARGO_PKG_REPOSITORY"),
    ")"
);

#[derive(Deserialize)]
pub struct CrateVersion {
    pub num: String,
    pub yanked: bool,
    pub rust_version: Option<String>,
}

#[derive(Deserialize)]
struct CratesResponse {
    versions: Vec<CrateVersion>,
}

pub fn fetch_crate_versions(crate_name: &str) -> Result<Vec<CrateVersion>> {
    let url = format!("https://crates.io/api/v1/crates/{crate_name}/versions?per_page=100");
    let response = reqwest::blocking::Client::new()
        .get(&url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .map_err(|source| Error::CratesIo {
            action: "querying crates.io",
            crate_name: crate_name.to_owned(),
            source,
        })?;
    let response: CratesResponse = response.json().map_err(|source| Error::CratesIo {
        action: "parsing crates.io's response",
        crate_name: crate_name.to_owned(),
        source,
    })?;
    Ok(response.versions)
}

/// Among non-yanked, non-pre-release `versions`, the highest one whose
/// `rust_version` does not exceed `toolchain_version` (or has none).
pub fn select_compatible_version(
    versions: &[CrateVersion],
    toolchain_version: &Version,
) -> Option<String> {
    let mut candidates: Vec<(Version, &str, Option<&str>)> = versions
        .iter()
        .filter(|v| !v.yanked)
        .filter_map(|v| {
            let parsed = parse_version(&v.num).ok()?;
            if !parsed.pre.is_empty() {
                return None;
            }
            Some((parsed, v.num.as_str(), v.rust_version.as_deref()))
        })
        .collect();

    candidates.sort_by(|a, b| b.0.cmp(&a.0));

    candidates.into_iter().find_map(|(_, num, rv_opt)| {
        let compatible = match rv_opt {
            None => true,
            Some(rv_str) => parse_version(rv_str).is_ok_and(|rv| rv <= *toolchain_version),
        };
        compatible.then(|| num.to_owned())
    })
}

pub fn resolve_compatible_crate_version(
    crate_name: &str,
    toolchain_version: &Version,
) -> Result<Option<String>> {
    let versions = fetch_crate_versions(crate_name)?;
    Ok(select_compatible_version(&versions, toolchain_version))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(num: &str, yanked: bool, rust_version: Option<&str>) -> CrateVersion {
        CrateVersion {
            num: num.to_owned(),
            yanked,
            rust_version: rust_version.map(str::to_owned),
        }
    }

    #[test]
    fn picks_highest_compatible_version() {
        let versions = vec![
            version("2.8.6", false, Some("1.82")),
            version("2.8.2", false, Some("1.75")),
            version("2.8.0", false, Some("1.70")),
        ];
        let toolchain = parse_version("1.75").unwrap();
        assert_eq!(
            select_compatible_version(&versions, &toolchain),
            Some("2.8.2".to_owned())
        );
    }

    #[test]
    fn skips_yanked_versions() {
        let versions = vec![
            version("2.8.2", true, Some("1.70")),
            version("2.8.0", false, Some("1.70")),
        ];
        let toolchain = parse_version("1.75").unwrap();
        assert_eq!(
            select_compatible_version(&versions, &toolchain),
            Some("2.8.0".to_owned())
        );
    }

    #[test]
    fn skips_pre_release_versions() {
        let versions = vec![
            version("2.9.0-beta.1", false, Some("1.70")),
            version("2.8.0", false, Some("1.70")),
        ];
        let toolchain = parse_version("1.75").unwrap();
        assert_eq!(
            select_compatible_version(&versions, &toolchain),
            Some("2.8.0".to_owned())
        );
    }

    #[test]
    fn treats_missing_rust_version_as_always_compatible() {
        let versions = vec![version("2.8.0", false, None)];
        let toolchain = parse_version("1.75").unwrap();
        assert_eq!(
            select_compatible_version(&versions, &toolchain),
            Some("2.8.0".to_owned())
        );
    }

    #[test]
    fn returns_none_when_nothing_fits() {
        let versions = vec![version("2.8.0", false, Some("1.80"))];
        let toolchain = parse_version("1.75").unwrap();
        assert_eq!(select_compatible_version(&versions, &toolchain), None);
    }
}
