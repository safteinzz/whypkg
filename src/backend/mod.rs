//! The cross-distro seam. A [`Backend`] knows how to interrogate one package
//! manager and produce a [`World`]; everything above this line (the engine, the
//! report, the TUI) is distro-agnostic. Adding pacman or dnf support means
//! implementing this one trait — nothing else changes.

use crate::model::World;
use std::process::Command;

pub mod apt;
pub mod dnf;
pub mod pacman;

/// One package-management ecosystem (apt/dpkg, pacman, dnf/rpm…).
pub trait Backend {
    /// Short label for messages, e.g. "apt".
    fn name(&self) -> &'static str;

    /// Run the bulk queries and assemble the whole [`World`] in one shot.
    fn build_world(&self) -> Result<World, String>;
}

/// Pick a backend for the current system by looking for its primary binary on
/// `PATH`. Order is just "most common first"; a machine should only have one.
pub fn detect() -> Option<Box<dyn Backend>> {
    if have("dpkg-query") || have("apt") {
        return Some(Box::new(apt::Apt));
    }
    if have("dnf") || have("rpm") {
        return Some(Box::new(dnf::Dnf));
    }
    if have("pacman") {
        return Some(Box::new(pacman::Pacman));
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

/// Run a command and return its stdout as a `String`, or an error describing
/// what failed. Used by backends for their bulk queries.
pub fn capture(bin: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(bin)
        .args(args)
        // Force the C locale so field labels, dates, and status messages are
        // predictable to parse regardless of the user's language settings.
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .map_err(|e| format!("could not run `{bin}`: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
