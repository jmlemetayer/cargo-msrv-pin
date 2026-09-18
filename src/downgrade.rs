//! The downgrade loop.
//!
//! Repeatedly finds registry packages whose declared MSRV exceeds the target
//! toolchain and downgrades them one at a time, climbing the dependency graph
//! when a package has no compatible version at all (see
//! `try_advance`/`unblock_unfixable_leaf`), until nothing is left to fix,
//! then verifies the result with `cargo check`.

use crate::command::command_run;
use crate::error::{Error, Result};
use crate::lockfile;
use crate::manifest;
use crate::metadata::{
    cargo_metadata, direct_parent_ids, find_package, incompatible_packages, package_named, root_id,
};
use crate::registry::{fetch_crate_versions, resolve_compatible_crate_version};
use crate::version::Toolchain;
use semver::Version;
use std::collections::{HashSet, VecDeque};

/// Safety net against a pathological dependency graph looping forever.
const MAX_UPDATES: usize = 100;

/// Safety net against climbing an unreasonably long or wide ancestor chain
/// while looking for a version to unblock an unfixable leaf.
const MAX_ANCESTOR_HOPS: usize = 20;

/// Stop descending a parent's version history after this many consecutive
/// failed attempts: that many failures in a row almost always means some
/// other package's requirement puts a floor under every version left to try
/// (e.g. a caret requirement on the parent that excludes every older
/// release), not that the next, older version might still work. Climbing to
/// the next ancestor is cheaper than grinding through the rest of a version
/// history that is blocked the same way throughout.
const MAX_CONSECUTIVE_DESCENT_FAILURES: usize = 3;

fn cargo_update(name: &str, version: &str, new_version: &str, toolchain: &Toolchain) -> Result<()> {
    let tc = toolchain.arg();
    let spec = format!("{name}@{version}");
    command_run(&["cargo", &tc, "update", &spec, "--precise", new_version])?;
    log::info!("updated {name} {version} -> {new_version}");
    Ok(())
}

fn cargo_check(toolchain: &Toolchain) -> Result<()> {
    log::info!("verifying build...");
    let tc = toolchain.arg();
    command_run(&["cargo", &tc, "check"])
}

/// A package's own Cargo.toml requirement can exclude the only compatible
/// version (e.g. a bare version string defaults to a caret requirement,
/// ruling out every version below it). Unlike a transitive conflict, no
/// later iteration ever clears that on its own, since the requirement lives
/// in the local package's manifest, not in another registry package.
fn relax_and_log(name: &str, new_version: &str) -> Result<()> {
    if manifest::relax_if_blocking(name, new_version)? {
        log::info!("relaxed {name}'s requirement in Cargo.toml to allow {new_version}");
    }
    Ok(())
}

/// Try every published version of `parent` older than `parent_version`,
/// newest first, until one no longer pulls in `leaf_name` at all. A newer
/// `parent` release can depend on a leaf with no compatible version at all
/// purely because it chose to use it, e.g. only for a particular target or
/// feature, not because of any version constraint, so an older release that
/// made a different choice can drop the dependency outright. Unlike
/// `select_compatible_version`, this doesn't filter candidates by their own
/// `rust_version`: we're not fixing `parent`'s MSRV here, only trying to
/// shed its dependency on `leaf_name`. If `parent`'s own `rust_version`
/// still ends up too high, the outer loop in `run` picks that up as
/// an ordinary incompatible package on its next pass.
fn descend_and_retry(
    parent: &str,
    parent_version: &str,
    leaf_name: &str,
    toolchain: &Toolchain,
) -> Result<bool> {
    let current = Version::parse(parent_version).map_err(|source| Error::InvalidVersion {
        version: parent_version.to_owned(),
        source,
    })?;
    let mut candidates: Vec<Version> = fetch_crate_versions(parent)?
        .iter()
        .filter(|v| !v.yanked)
        .filter_map(|v| Version::parse(&v.num).ok())
        .filter(|v| v.pre.is_empty() && *v < current)
        .collect();
    candidates.sort_by(|a, b| b.cmp(a));

    let mut locked_at = parent_version.to_owned();
    let mut consecutive_failures = 0;

    for candidate in candidates {
        if consecutive_failures >= MAX_CONSECUTIVE_DESCENT_FAILURES {
            break;
        }
        let candidate = candidate.to_string();
        relax_and_log(parent, &candidate)?;
        if let Err(e) = cargo_update(parent, &locked_at, &candidate, toolchain) {
            if !matches!(e, Error::Command(_)) {
                return Err(e);
            }
            consecutive_failures += 1;
            continue;
        }

        // A precise update only pins the lock entry: it doesn't validate
        // that everything the root package needs from `parent` (e.g. a
        // feature only present in newer releases) still exists. If that
        // now breaks metadata resolution, undo it immediately rather than
        // leave Cargo.lock sitting on a version that cannot build, then
        // treat it like any other failed candidate.
        match cargo_metadata(None) {
            Ok(metadata) => {
                if package_named(&metadata, leaf_name).is_none() {
                    return Ok(true);
                }
                locked_at = candidate;
                consecutive_failures = 0;
            }
            Err(_) => {
                cargo_update(parent, &candidate, &locked_at, toolchain)?;
                consecutive_failures += 1;
            }
        }
    }

    Ok(false)
}

/// `leaf_name` has no crates.io version compatible with `toolchain` at all.
/// Rather than give up immediately, climb the dependency graph: a package
/// isn't itself flagged incompatible just because it depends on one, but a
/// newer release of it can still be the actual reason an unfixable leaf is
/// in the graph at all. Try downgrading each direct parent (`descend_and_retry`);
/// when a parent's own history never drops the leaf either, climb past it to
/// its own parents in turn, since removing the leaf then depends on that
/// grandparent no longer needing the current version of the parent.
fn unblock_unfixable_leaf(leaf_name: &str, toolchain: &Toolchain) -> Result<bool> {
    let mut visited: HashSet<String> = HashSet::new();
    // A queue, not a stack: explore one hop's worth of parents before
    // climbing to the next hop, so a nearby fix (e.g. two hops up) is found
    // before a distant, unrelated branch (fan-in siblings can lead anywhere)
    // is ever touched.
    let mut frontier: VecDeque<String> = VecDeque::from([leaf_name.to_owned()]);

    for _ in 0..MAX_ANCESTOR_HOPS {
        let Some(name) = frontier.pop_front() else {
            return Ok(false);
        };
        if !visited.insert(name.clone()) {
            continue;
        }

        let metadata = cargo_metadata(None)?;
        if package_named(&metadata, leaf_name).is_none() {
            return Ok(true); // already unblocked by an earlier branch
        }
        let Some(pkg) = package_named(&metadata, &name) else {
            continue; // `name` is already gone; nothing left to climb from here
        };

        for parent_id in direct_parent_ids(&metadata, &pkg.id) {
            if root_id(&metadata) == Some(parent_id) {
                continue; // the local package itself isn't a version to try
            }
            let Some(parent) = find_package(&metadata, parent_id) else {
                continue;
            };
            if parent.source.is_none() {
                continue; // path or git dependency: no registry history to descend
            }
            if descend_and_retry(&parent.name, &parent.version, leaf_name, toolchain)? {
                return Ok(true);
            }
            frontier.push_back(parent.name.clone());
        }
    }

    Ok(false)
}

/// Try to make progress on one incompatible package: downgrade it directly,
/// or, when it has no compatible version at all, climb the dependency graph
/// to unblock it instead. Reports whether either one advanced the graph.
fn try_advance(name: &str, version: &str, toolchain: &Toolchain) -> Result<bool> {
    let new_version = match resolve_compatible_crate_version(name, &toolchain.version)? {
        Some(v) => v,
        None => {
            log::info!("no compatible version for {name} {version}, climbing the dependency graph");
            if unblock_unfixable_leaf(name, toolchain)? {
                return Ok(true);
            }
            log::warn!("no compatible version found for {name} {version}, skipping");
            return Ok(false);
        }
    };
    relax_and_log(name, &new_version)?;
    match cargo_update(name, version, &new_version, toolchain) {
        Ok(()) => Ok(true),
        Err(Error::Command(_)) => {
            log::debug!("update of {name}@{version} deferred due to version conflict");
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// Downgrade registry dependencies until every one of them satisfies
/// `toolchain`, then verify the build with `cargo +<toolchain> check`.
pub fn run(toolchain: &str) -> Result<()> {
    let toolchain = Toolchain::parse(toolchain)?;
    lockfile::ensure_readable_by(&toolchain.raw)?;
    let mut exhausted = true;

    for _ in 0..MAX_UPDATES {
        let metadata = cargo_metadata(None)?;
        let incompatible = incompatible_packages(&metadata, &toolchain.version);

        if incompatible.is_empty() {
            exhausted = false;
            break;
        }

        log::info!("{} incompatible package(s) remaining", incompatible.len());

        // Try each incompatible package in turn and stop at the first one
        // that makes progress. The rest are left for a later iteration: a
        // tight version constraint from another incompatible package (its
        // own newer release requires a newer version of this one) unblocks
        // once that other package is downgraded first, and a package
        // `try_advance` couldn't unblock at all is left to `cargo check` to
        // determine whether it's actually compiled for the current target.
        let mut made_progress = false;
        for (name, version) in &incompatible {
            if try_advance(name, version, &toolchain)? {
                made_progress = true;
                break;
            }
        }

        if !made_progress {
            let names: Vec<&str> = incompatible.iter().map(|(name, _)| name.as_str()).collect();
            log::warn!(
                "could not downgrade the remaining incompatible package(s): {}",
                names.join(", ")
            );
            exhausted = false;
            break;
        }
    }

    if exhausted {
        log::warn!("reached the {MAX_UPDATES}-update safety limit without resolving every package");
    }

    cargo_check(&toolchain)?;
    log::info!("done");
    Ok(())
}
