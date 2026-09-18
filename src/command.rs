//! External command execution.
//!
//! Runs external commands (cargo, mainly) and reports failures with their
//! captured exit status, stdout, and stderr via `CommandFailure`.

use crate::error::{Error, Result};
use std::process::{Command, Output};

#[derive(Debug)]
pub struct CommandFailure {
    args: Vec<String>,
    returncode: i32,
    stdout: Vec<String>,
    stderr: Vec<String>,
}

impl std::fmt::Display for CommandFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "`{}` failed with exit status {}",
            self.args.join(" "),
            self.returncode
        )?;
        if !self.stdout.is_empty() {
            writeln!(f, "stdout:\n{}", self.stdout.join("\n"))?;
        }
        if !self.stderr.is_empty() {
            write!(f, "stderr:\n{}", self.stderr.join("\n"))?;
        }
        Ok(())
    }
}

impl std::error::Error for CommandFailure {}

impl CommandFailure {
    pub fn stderr_contains(&self, needle: &str) -> bool {
        self.stderr.iter().any(|line| line.contains(needle))
    }
}

fn run(args: &[&str]) -> Result<Output> {
    log::debug!("COMMAND: {}", args.join(" "));
    Command::new(args[0])
        .args(&args[1..])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::CommandNotFound(args[0].to_owned())
            } else {
                Error::Io {
                    action: "spawning a command",
                    source: e,
                }
            }
        })
}

fn failure(args: &[&str], output: &Output) -> Error {
    Error::Command(CommandFailure {
        args: args.iter().map(|&s| s.to_owned()).collect(),
        returncode: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect(),
        stderr: String::from_utf8_lossy(&output.stderr)
            .lines()
            .map(str::to_owned)
            .collect(),
    })
}

pub fn command_run(args: &[&str]) -> Result<()> {
    let output = run(args)?;
    if !output.status.success() {
        return Err(failure(args, &output));
    }
    Ok(())
}

pub fn command_output(args: &[&str]) -> Result<String> {
    let output = run(args)?;
    if !output.status.success() {
        return Err(failure(args, &output));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(stderr: &[&str]) -> CommandFailure {
        CommandFailure {
            args: vec![],
            returncode: 1,
            stdout: vec![],
            stderr: stderr.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn stderr_contains_matches_substring_in_any_line() {
        let f = failure(&["some other line", "lock file version `4` was found"]);
        assert!(f.stderr_contains("lock file version"));
    }

    #[test]
    fn stderr_contains_false_when_absent() {
        let f = failure(&["totally unrelated error"]);
        assert!(!f.stderr_contains("lock file version"));
    }
}
