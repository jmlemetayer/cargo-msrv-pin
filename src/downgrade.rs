//! The downgrade loop.
//!
//! Repeatedly finds registry packages whose declared MSRV exceeds the target
//! toolchain and downgrades them one at a time, until nothing is left to fix,
//! then verifies the result with `cargo check`.

use crate::command::command_run;
use crate::error::{Error, Result};
use crate::metadata::{cargo_metadata, incompatible_packages};
use crate::registry::resolve_compatible_crate_version;
use crate::version::Toolchain;

/// Safety net against a pathological dependency graph looping forever.
const MAX_UPDATES: usize = 100;

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

/// Try to make progress on one incompatible package by downgrading it
/// directly. Reports whether it succeeded.
fn try_advance(name: &str, version: &str, toolchain: &Toolchain) -> Result<bool> {
    let new_version = match resolve_compatible_crate_version(name, &toolchain.version)? {
        Some(v) => v,
        None => {
            log::warn!("no compatible version found for {name} {version}, skipping");
            return Ok(false);
        }
    };
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
        // once that other package is downgraded first.
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
