//! `diffyman git install`: git configuration that makes diffyman a
//! difftool and a textconv driver.
//!
//! Without `--global` or `--local` nothing is written: the commands are
//! printed. With one of them, `git config` writes the settings to that scope.
//! `--dry-run` prints instead of writing, whatever the scope.

use std::io::Write;
use std::process::Command;

use clap::{ArgGroup, Args};

use crate::output::{Failure, Sink};

/// Arguments of `git install`.
#[derive(Debug, Args)]
#[command(group(ArgGroup::new("scope").args(["global", "local"])))]
pub struct InstallArgs {
    /// Write to the global git config (`git config --global`).
    #[arg(long)]
    pub global: bool,
    /// Write to the current repository's config (`git config --local`).
    #[arg(long)]
    pub local: bool,
    /// Print the commands instead of running them (the default when no
    /// scope is given).
    #[arg(long)]
    pub dry_run: bool,
    /// The command git runs for diffyman, if `diffyman` is not on the PATH.
    #[arg(long, value_name = "PATH", default_value = "diffyman")]
    pub program: String,
}

/// Quotes a value for a POSIX shell when it needs quoting.
fn shell_quote(value: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "-_./=:@+,".contains(c);
    if !value.is_empty() && value.chars().all(plain) {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

/// The settings, in the order they are written.
pub fn settings(program: &str) -> Vec<(&'static str, String)> {
    let program = shell_quote(program);
    vec![
        ("diff.tool", "diffyman".to_owned()),
        (
            "difftool.diffyman.cmd",
            format!(r#"{program} diff "$LOCAL" "$REMOTE""#),
        ),
        ("diff.diffyman.textconv", format!("{program} textconv")),
    ]
}

/// Prints or applies the settings.
pub fn install(args: &InstallArgs) -> Result<u8, Failure> {
    let scope = if args.global {
        Some("--global")
    } else if args.local {
        Some("--local")
    } else {
        None
    };
    let settings = settings(&args.program);
    let mut out = Sink::open(None)?;
    match scope {
        Some(scope) if !args.dry_run => {
            for (key, value) in &settings {
                let status = Command::new("git")
                    .args(["config", scope, key, value])
                    .status()
                    .map_err(|e| format!("cannot run git: {e}"))?;
                if !status.success() {
                    return Err(Failure::Message(format!(
                        "git config {scope} {key} failed ({status})"
                    )));
                }
                writeln!(out, "git config {scope} {key} {}", shell_quote(value))?;
            }
            writeln!(
                out,
                "\nDone. Compare with: git difftool <commit>   (folders: git difftool -d)"
            )?;
        }
        _ => {
            writeln!(out, "# Dry run: nothing was written.")?;
            if scope.is_none() {
                writeln!(
                    out,
                    "# Run `diffyman git install --global` (or --local inside a repository) to apply:"
                )?;
            }
            let scope = scope.unwrap_or("--global");
            for (key, value) in &settings {
                writeln!(out, "git config {scope} {key} {}", shell_quote(value))?;
            }
            writeln!(out, "#")?;
            writeln!(
                out,
                "# Then: git difftool <commit>   (folders: git difftool -d)"
            )?;
            writeln!(
                out,
                "# Textconv: add `*.txt diff=diffyman` to .gitattributes to compare what the text adapter compares."
            )?;
        }
    }
    out.finish()?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("diffyman"), "diffyman");
        assert_eq!(shell_quote("/opt/bin/diffyman"), "/opt/bin/diffyman");
        assert_eq!(shell_quote("my tools/diffyman"), "'my tools/diffyman'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(
            settings("diffyman")[1].1,
            r#"diffyman diff "$LOCAL" "$REMOTE""#
        );
    }
}
