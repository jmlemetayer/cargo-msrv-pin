//! Cargo.toml editing.
//!
//! Relaxes a direct dependency's version requirement when it is the reason a
//! downgrade can never succeed, rather than a transient ordering issue that
//! later clears on its own. Uses `toml_edit` so only the touched requirement
//! changes, not the rest of the file's formatting.

use crate::error::{Error, Result};
use semver::{Version, VersionReq};
use std::fs;
use toml_edit::{DocumentMut, Item, Value};

const MANIFEST_FILE: &str = "Cargo.toml";
const DEPENDENCY_TABLES: &[&str] = &["dependencies", "dev-dependencies", "build-dependencies"];

/// The item actually holding the version requirement string for a dependency
/// entry: the entry itself for `name = "req"`, or its `version` key for
/// `name = { version = "req", ... }` and `[dependencies.name]` forms.
fn requirement_item(dep_item: &mut Item) -> Option<&mut Item> {
    if dep_item.is_str() {
        Some(dep_item)
    } else {
        dep_item.as_table_like_mut()?.get_mut("version")
    }
}

/// If `name`'s requirement in `doc` excludes `new_version`, pin it there
/// directly (preserving the rest of the entry, e.g. `features`) and report
/// `true`. A requirement that already allows `new_version` is left alone.
fn relax_requirement(doc: &mut DocumentMut, name: &str, new_version: &str) -> Result<bool> {
    let target = Version::parse(new_version).map_err(|source| Error::InvalidVersion {
        version: new_version.to_owned(),
        source,
    })?;

    for table_name in DEPENDENCY_TABLES {
        let Some(table) = doc.get_mut(table_name).and_then(Item::as_table_like_mut) else {
            continue;
        };
        let Some(dep_item) = table.get_mut(name) else {
            continue;
        };
        let Some(req_item) = requirement_item(dep_item) else {
            continue;
        };
        let Some(req_str) = req_item.as_str().map(str::to_owned) else {
            continue;
        };
        let excludes = VersionReq::parse(&req_str).is_ok_and(|req| !req.matches(&target));
        if !excludes {
            continue;
        }

        let value = req_item.as_value_mut().expect("string item has a value");
        let decor = value.decor().clone();
        *value = Value::from(format!("={new_version}"));
        *value.decor_mut() = decor;
        return Ok(true);
    }

    Ok(false)
}

/// If `name` is a direct dependency in `Cargo.toml` whose own requirement
/// excludes `new_version`, pin it to `new_version` and report `true`.
///
/// This is what makes `cargo update --precise` fail forever rather than on a
/// transient ordering issue: the requirement lives in the local package's own
/// manifest, so no other package's later update can ever relax it.
pub fn relax_if_blocking(name: &str, new_version: &str) -> Result<bool> {
    let content = fs::read_to_string(MANIFEST_FILE).map_err(|source| Error::Io {
        action: "reading Cargo.toml",
        source,
    })?;
    let mut doc: DocumentMut = content.parse().map_err(Error::ParseManifest)?;

    if !relax_requirement(&mut doc, name, new_version)? {
        return Ok(false);
    }

    fs::write(MANIFEST_FILE, doc.to_string()).map_err(|source| Error::Io {
        action: "writing Cargo.toml",
        source,
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_a_plain_string_requirement() {
        let mut doc: DocumentMut = "[dependencies]\nclap = \"4.6.7\"\n".parse().unwrap();
        assert!(relax_requirement(&mut doc, "clap", "4.5.61").unwrap());
        assert_eq!(doc.to_string(), "[dependencies]\nclap = \"=4.5.61\"\n");
    }

    #[test]
    fn pins_the_version_key_of_an_inline_table_and_keeps_its_other_keys() {
        let mut doc: DocumentMut =
            "[dependencies]\nclap = { version = \"4.6.7\", features = [\"derive\"] }\n"
                .parse()
                .unwrap();
        assert!(relax_requirement(&mut doc, "clap", "4.5.61").unwrap());
        assert_eq!(
            doc.to_string(),
            "[dependencies]\nclap = { version = \"=4.5.61\", features = [\"derive\"] }\n"
        );
    }

    #[test]
    fn pins_the_version_key_of_a_dotted_table() {
        let mut doc: DocumentMut = "[dependencies.clap]\nversion = \"4.6.7\"\n"
            .parse()
            .unwrap();
        assert!(relax_requirement(&mut doc, "clap", "4.5.61").unwrap());
        assert_eq!(
            doc.to_string(),
            "[dependencies.clap]\nversion = \"=4.5.61\"\n"
        );
    }

    #[test]
    fn leaves_a_requirement_that_already_allows_the_new_version() {
        let mut doc: DocumentMut = "[dependencies]\nclap = \">=4.5.0\"\n".parse().unwrap();
        assert!(!relax_requirement(&mut doc, "clap", "4.5.61").unwrap());
        assert_eq!(doc.to_string(), "[dependencies]\nclap = \">=4.5.0\"\n");
    }

    #[test]
    fn ignores_a_package_that_is_not_a_direct_dependency() {
        let mut doc: DocumentMut = "[dependencies]\nclap = \"4.6.7\"\n".parse().unwrap();
        assert!(!relax_requirement(&mut doc, "clap_builder", "4.5.61").unwrap());
        assert_eq!(doc.to_string(), "[dependencies]\nclap = \"4.6.7\"\n");
    }

    #[test]
    fn checks_dev_and_build_dependencies_too() {
        let mut doc: DocumentMut = "[build-dependencies]\nclap = \"4.6.7\"\n".parse().unwrap();
        assert!(relax_requirement(&mut doc, "clap", "4.5.61").unwrap());
        assert_eq!(
            doc.to_string(),
            "[build-dependencies]\nclap = \"=4.5.61\"\n"
        );
    }
}
