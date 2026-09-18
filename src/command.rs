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

    #[test]
    fn displays_the_command_exit_status_and_captured_output() {
        let f = CommandFailure {
            args: vec!["cargo".to_owned(), "check".to_owned()],
            returncode: 101,
            stdout: vec![],
            stderr: vec!["error: something went wrong".to_owned()],
        };
        assert_eq!(
            f.to_string(),
            "`cargo check` failed with exit status 101\nstderr:\nerror: something went wrong"
        );
    }
}
