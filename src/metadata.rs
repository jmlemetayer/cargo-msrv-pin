//! `cargo metadata` reading and interpretation.
//!
//! Finds which registry packages exceed the target toolchain's MSRV.

use crate::command::command_output;
use crate::error::{Error, Result};
use crate::version::parse_version;
use semver::Version;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct MetadataPackage {
    pub name: String,
    pub version: String,
    pub source: Option<String>,
    pub rust_version: Option<String>,
}

#[derive(Deserialize)]
pub struct CargoMetadata {
    pub packages: Vec<MetadataPackage>,
}

/// With `toolchain`, runs under `cargo +<toolchain>` instead of the
/// default toolchain.
pub fn cargo_metadata(toolchain: Option<&str>) -> Result<CargoMetadata> {
    let toolchain_arg = toolchain.map(|tc| format!("+{tc}"));
    let mut args: Vec<&str> = vec!["cargo"];
    if let Some(tc) = toolchain_arg.as_deref() {
        args.push(tc);
    }
    args.extend(["metadata", "--format-version", "1"]);

    let stdout = command_output(&args)?;
    serde_json::from_str(&stdout).map_err(Error::Metadata)
}

/// Registry packages whose `rust_version` exceeds `toolchain_version`, as
/// `(name, version)` pairs.
pub fn incompatible_packages(
    metadata: &CargoMetadata,
    toolchain_version: &Version,
) -> Vec<(String, String)> {
    let mut incompatible = Vec::new();

    for pkg in &metadata.packages {
        let is_registry = pkg
            .source
            .as_deref()
            .is_some_and(|s| s.starts_with("registry+"));
        if !is_registry {
            continue;
        }
        let Some(rv_str) = &pkg.rust_version else {
            continue;
        };
        match parse_version(rv_str) {
            Ok(rv) if rv > *toolchain_version => {
                log::debug!(
                    "package {} {} requires Rust {} > toolchain {}",
                    pkg.name,
                    pkg.version,
                    rv_str,
                    toolchain_version
                );
                incompatible.push((pkg.name.clone(), pkg.version.clone()));
            }
            Ok(_) => {}
            Err(e) => {
                log::warn!(
                    "could not parse rust_version '{}' for {}: {e}",
                    rv_str,
                    pkg.name
                );
            }
        }
    }

    incompatible
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(
        name: &str,
        version: &str,
        source: Option<&str>,
        rust_version: Option<&str>,
    ) -> MetadataPackage {
        MetadataPackage {
            name: name.to_owned(),
            version: version.to_owned(),
            source: source.map(str::to_owned),
            rust_version: rust_version.map(str::to_owned),
        }
    }

    #[test]
    fn flags_packages_above_toolchain() {
        let metadata = CargoMetadata {
            packages: vec![
                pkg(
                    "clap",
                    "4.6.6",
                    Some("registry+https://github.com/rust-lang/crates.io-index"),
                    Some("1.80"),
                ),
                pkg(
                    "serde",
                    "1.0.229",
                    Some("registry+https://github.com/rust-lang/crates.io-index"),
                    Some("1.70"),
                ),
            ],
        };
        let toolchain = parse_version("1.75").unwrap();
        let incompatible = incompatible_packages(&metadata, &toolchain);
        assert_eq!(incompatible, vec![("clap".to_owned(), "4.6.6".to_owned())]);
    }

    #[test]
    fn ignores_path_and_git_dependencies() {
        let metadata = CargoMetadata {
            packages: vec![
                pkg("my-workspace-crate", "0.1.0", None, Some("1.80")),
                pkg(
                    "some-git-dep",
                    "0.1.0",
                    Some("git+https://example.com/dep"),
                    Some("1.80"),
                ),
            ],
        };
        let toolchain = parse_version("1.75").unwrap();
        assert!(incompatible_packages(&metadata, &toolchain).is_empty());
    }

    #[test]
    fn ignores_packages_without_rust_version() {
        let metadata = CargoMetadata {
            packages: vec![pkg(
                "no-msrv",
                "0.1.0",
                Some("registry+https://github.com/rust-lang/crates.io-index"),
                None,
            )],
        };
        let toolchain = parse_version("1.75").unwrap();
        assert!(incompatible_packages(&metadata, &toolchain).is_empty());
    }

    #[test]
    fn skips_unparsable_rust_version_with_a_warning() {
        let metadata = CargoMetadata {
            packages: vec![pkg(
                "weird-msrv",
                "0.1.0",
                Some("registry+https://github.com/rust-lang/crates.io-index"),
                Some("not-a-version"),
            )],
        };
        let toolchain = parse_version("1.75").unwrap();
        assert!(incompatible_packages(&metadata, &toolchain).is_empty());
    }
}
