//! The cross-distro seam. A [`Backend`] knows how to interrogate one package
//! manager and produce a [`World`]; everything above this line (the engine, the
//! report, the TUI) is distro-agnostic. Adding pacman or dnf support means
//! implementing this one trait - nothing else changes.

use crate::model::World;
use std::io::ErrorKind;
use std::process::{Command, Output};

pub mod apt;
pub mod dnf;
pub mod flatpak;
pub mod pacman;
pub mod zypper;

/// One package-management ecosystem (apt/dpkg, pacman, dnf/rpm…).
pub trait Backend {
    /// Short label for messages, e.g. "apt".
    fn name(&self) -> &'static str;

    /// Run the bulk queries and assemble the whole [`World`] in one shot.
    fn build_world(&self) -> Result<World, String>;

    /// The auto-installed packages nothing needs anymore, as the package
    /// manager itself decides it (what its autoremove would take). The graph
    /// cannot answer this, because a dependency kind it does not model (a
    /// Suggests, a virtual package, an alternative) would make a needed package
    /// look removable. Read-only and offline, like every other query.
    fn unneeded(&self) -> Result<Vec<String>, String>;
}

/// Pick a backend for the current system by looking for its package manager on
/// `PATH`. Distros package each other's managers (Fedora ships `zypper`, Arch
/// ships `rpm`), so each front end is checked before the bare `rpm` they share.
pub fn detect() -> Option<Box<dyn Backend>> {
    if have("dpkg-query") || have("apt") {
        return Some(Box::new(apt::Apt));
    }
    if have("pacman") {
        return Some(Box::new(pacman::Pacman));
    }
    if have("dnf") {
        return Some(Box::new(dnf::Dnf));
    }
    if have("zypper") {
        return Some(Box::new(zypper::Zypper));
    }
    if have("rpm") {
        return Some(Box::new(dnf::Dnf));
    }
    None
}

/// Whether a command exists on `PATH` (via `command -v`).
fn have(bin: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {bin}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run a command and return its stdout as a `String`, whatever its exit code.
/// Used for the bulk queries whose tools exit non-zero for an empty answer;
/// fails only when the command cannot start.
pub fn capture(bin: &str, args: &[&str]) -> Result<String, String> {
    let out = run(bin, args)?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Like [`capture`], but an exit code outside `ok` (and other than 0) is an
/// error quoting what the command said on stderr, so a failed query is never
/// read as an empty answer.
pub fn capture_checked(bin: &str, args: &[&str], ok: &[i32]) -> Result<String, String> {
    let out = run(bin, args)?;
    let code = out.status.code();
    if code == Some(0) || code.is_some_and(|c| ok.contains(&c)) {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let said: Vec<String> = String::from_utf8_lossy(&out.stderr)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(5)
        .map(String::from)
        .collect();
    let status = match code {
        Some(c) => format!("exited {c}"),
        None => "was killed".to_string(),
    };
    let cmd = std::iter::once(bin)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    if said.is_empty() {
        Err(format!("`{cmd}` {status}"))
    } else {
        Err(format!("`{cmd}` {status}:\n  {}", said.join("\n  ")))
    }
}

fn run(bin: &str, args: &[&str]) -> Result<Output, String> {
    Command::new(bin)
        .args(args)
        // Force the C locale so field labels, dates, and status messages are
        // predictable to parse regardless of the user's language settings.
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => format!("`{bin}` is not installed or not on PATH"),
            ErrorKind::PermissionDenied => format!("`{bin}` is not executable by you"),
            _ => format!("could not start `{bin}`"),
        })
}
