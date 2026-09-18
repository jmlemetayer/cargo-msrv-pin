//! `cargo metadata` reading and interpretation.
//!
//! Finds which registry packages exceed the target toolchain's MSRV
//! (`incompatible_packages`), and, via the resolve graph, which packages
//! depend on which (`direct_parent_ids`).

use crate::command::command_output;
use crate::error::{Error, Result};
use crate::version::parse_version;
use semver::Version;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct MetadataPackage {
    pub name: String,
    pub version: String,
    pub id: String,
    pub source: Option<String>,
    pub rust_version: Option<String>,
}

#[derive(Deserialize)]
struct ResolveNodeDep {
    pkg: String,
}

#[derive(Deserialize)]
struct ResolveNode {
    id: String,
    deps: Vec<ResolveNodeDep>,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<ResolveNode>,
    root: Option<String>,
}

#[derive(Deserialize)]
pub struct CargoMetadata {
    pub packages: Vec<MetadataPackage>,
    resolve: Resolve,
}

/// The id of the local package `cargo metadata` was run against, or `None`
/// for a virtual workspace with no root package.
pub fn root_id(metadata: &CargoMetadata) -> Option<&str> {
    metadata.resolve.root.as_deref()
}

/// The package with the given id, if any.
pub fn find_package<'a>(metadata: &'a CargoMetadata, id: &str) -> Option<&'a MetadataPackage> {
    metadata.packages.iter().find(|p| p.id == id)
}

/// The package with the given name, if any is currently in the graph.
pub fn package_named<'a>(metadata: &'a CargoMetadata, name: &str) -> Option<&'a MetadataPackage> {
    metadata.packages.iter().find(|p| p.name == name)
}

/// Ids of the packages that declare a direct dependency on `id`.
///
/// This is `cargo metadata`'s resolve graph read backwards: it only records
/// forward edges (a package's own dependencies), so finding what depends on
/// a given package means scanning every node for an edge that targets it.
pub fn direct_parent_ids<'a>(metadata: &'a CargoMetadata, id: &str) -> Vec<&'a str> {
    metadata
        .resolve
        .nodes
        .iter()
        .filter(|node| node.deps.iter().any(|dep| dep.pkg == id))
        .map(|node| node.id.as_str())
        .collect()
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
            id: format!("id#{name}@{version}"),
            source: source.map(str::to_owned),
            rust_version: rust_version.map(str::to_owned),
        }
    }

    fn metadata(packages: Vec<MetadataPackage>) -> CargoMetadata {
        CargoMetadata {
            packages,
            resolve: Resolve {
                nodes: vec![],
                root: None,
            },
        }
    }

    #[test]
    fn flags_packages_above_toolchain() {
        let metadata = metadata(vec![
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
        ]);
        let toolchain = parse_version("1.75").unwrap();
        let incompatible = incompatible_packages(&metadata, &toolchain);
        assert_eq!(incompatible, vec![("clap".to_owned(), "4.6.6".to_owned())]);
    }

    #[test]
    fn ignores_path_and_git_dependencies() {
        let metadata = metadata(vec![
            pkg("my-workspace-crate", "0.1.0", None, Some("1.80")),
            pkg(
                "some-git-dep",
                "0.1.0",
                Some("git+https://example.com/dep"),
                Some("1.80"),
            ),
        ]);
        let toolchain = parse_version("1.75").unwrap();
        assert!(incompatible_packages(&metadata, &toolchain).is_empty());
    }

    #[test]
    fn ignores_packages_without_rust_version() {
        let metadata = metadata(vec![pkg(
            "no-msrv",
            "0.1.0",
            Some("registry+https://github.com/rust-lang/crates.io-index"),
            None,
        )]);
        let toolchain = parse_version("1.75").unwrap();
        assert!(incompatible_packages(&metadata, &toolchain).is_empty());
    }

    #[test]
    fn skips_unparsable_rust_version_with_a_warning() {
        let metadata = metadata(vec![pkg(
            "weird-msrv",
            "0.1.0",
            Some("registry+https://github.com/rust-lang/crates.io-index"),
            Some("not-a-version"),
        )]);
        let toolchain = parse_version("1.75").unwrap();
        assert!(incompatible_packages(&metadata, &toolchain).is_empty());
    }

    #[test]
    fn finds_the_direct_parent_of_a_package() {
        let mut metadata = metadata(vec![
            pkg("wasip2", "1.0.4", Some("registry+..."), Some("1.87")),
            pkg("getrandom", "0.3.4", Some("registry+..."), Some("1.70")),
            pkg("tempfile", "3.27.0", Some("registry+..."), Some("1.70")),
        ]);
        metadata.resolve = Resolve {
            root: Some("id#kerosd@1.0.0".to_owned()),
            nodes: vec![
                ResolveNode {
                    id: "id#getrandom@0.3.4".to_owned(),
                    deps: vec![ResolveNodeDep {
                        pkg: "id#wasip2@1.0.4".to_owned(),
                    }],
                },
                ResolveNode {
                    id: "id#tempfile@3.27.0".to_owned(),
                    deps: vec![ResolveNodeDep {
                        pkg: "id#getrandom@0.3.4".to_owned(),
                    }],
                },
            ],
        };

        assert_eq!(
            direct_parent_ids(&metadata, "id#wasip2@1.0.4"),
            vec!["id#getrandom@0.3.4"]
        );
        assert_eq!(
            find_package(&metadata, "id#getrandom@0.3.4").map(|p| p.name.as_str()),
            Some("getrandom")
        );
        assert_eq!(root_id(&metadata), Some("id#kerosd@1.0.0"));
    }

    #[test]
    fn finds_a_package_by_name_or_reports_it_absent() {
        let metadata = metadata(vec![pkg(
            "wasip2",
            "1.0.4",
            Some("registry+..."),
            Some("1.87"),
        )]);
        assert_eq!(
            package_named(&metadata, "wasip2").map(|p| p.version.as_str()),
            Some("1.0.4")
        );
        assert!(package_named(&metadata, "getrandom").is_none());
    }

    #[test]
    fn reports_no_parents_for_a_package_nothing_depends_on() {
        let metadata = metadata(vec![pkg(
            "standalone",
            "1.0.0",
            Some("registry+..."),
            Some("1.70"),
        )]);
        assert!(direct_parent_ids(&metadata, "id#standalone@1.0.0").is_empty());
    }
}
