//! whypkg - why the hell is this package here?
//!
//! A fast, cross-distro package investigator. Two modes share one engine:
//!   whypkg                 Interactive browser: fuzzy-find a package, open its
//!                          dossier, drill through what needs it / what it needs.
//!   whypkg --upgradable    Same browser, scoped to packages with pending upgrades.
//!   whypkg pending         A grouped report of every pending upgrade and *why*
//!                          it's on your system (kernel / your apps / pulled in by…).
//!   whypkg self update     Reinstall the latest release from crates.io.
//!   whypkg self check      Ask crates.io whether a newer release exists.
//!
//! The package-manager specifics live behind a single `Backend` trait, so apt,
//! pacman, and dnf all feed the same analysis and the same UI.

mod backend;
mod commands;
mod engine;
mod model;
mod tui;

use clap::{Parser, Subcommand};

const EXAMPLES: &str = concat!(
    "\x1b[1mExamples:\x1b[0m
  whypkg                     Browse every installed package
  whypkg --upgradable        Browse only packages with a pending upgrade
  whypkg pending             Report every pending upgrade, grouped by why it's here
  whypkg pending --quick     One line per pending package: size + reason
  whypkg self update         Reinstall the latest release from crates.io
  whypkg self check          Ask crates.io whether a newer release exists

Inside the browser: type to filter, Enter to open, Esc to go back.",
    "\n\n",
    env!("CARGO_PKG_REPOSITORY"),
    "\ncontributors: ",
    env!("CARGO_PKG_AUTHORS"),
);

/// `-V` stays a bare version string for scripts; `--version` spells out the
/// license, where it lives, and who's contributed. Every field comes from
/// Cargo.toml, so none of it can drift from the manifest.
const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\n",
    env!("CARGO_PKG_LICENSE"),
    "  ",
    env!("CARGO_PKG_REPOSITORY"),
    "\ncontributors: ",
    env!("CARGO_PKG_AUTHORS"),
);

#[derive(Parser)]
#[command(
    name = "whypkg",
    bin_name = "whypkg",
    version,
    long_version = LONG_VERSION,
    about,
    after_help = EXAMPLES,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,

    /// (browse mode) Limit the browser to packages with a pending upgrade
    #[arg(long, global = true)]
    upgradable: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Report every pending upgrade, grouped by why it's on your system
    Pending(commands::pending::Args),
    /// Manage whypkg itself: `self update` reinstalls, `self check` looks for a newer release
    #[command(name = "self", subcommand)]
    Selfie(commands::selfcmd::Cmd),
}

/// Rust starts with `SIGPIPE` ignored, so writing to a closed pipe returns an
/// error and `println!` panics. That makes `whypkg pending --quick | head`
/// explode instead of ending quietly. Restore the default so we behave like
/// every other Unix tool and just die when the reader goes away.
#[cfg(unix)]
fn restore_sigpipe() {
    // SAFETY: setting a signal disposition to the default is always sound, and
    // this runs before any threads exist.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_sigpipe() {}

fn main() {
    restore_sigpipe();
    let cli = Cli::parse();

    match cli.command {
        Some(Cmd::Pending(args)) => commands::pending::run(args),
        Some(Cmd::Selfie(cmd)) => commands::selfcmd::run(cmd),
        // No subcommand → the interactive browser (the heart of the tool).
        None => commands::browse::run(commands::browse::Args {
            upgradable: cli.upgradable,
        }),
    }
}
