//! whypkg — why the hell is this package here?
//!
//! A fast, cross-distro package investigator. Two modes share one engine:
//!   whypkg                 Interactive browser: fuzzy-find a package, open its
//!                          dossier, drill through what needs it / what it needs.
//!   whypkg --upgradable    Same browser, scoped to packages with pending upgrades.
//!   whypkg pending         A grouped report of every pending upgrade and *why*
//!                          it's on your system (kernel / your apps / pulled in by…).
//!   whypkg update          Update whypkg itself to the latest release.
//!
//! The package-manager specifics live behind a single `Backend` trait, so apt,
//! pacman, and dnf all feed the same analysis and the same UI.

mod backend;
mod commands;
mod engine;
mod model;

use clap::{Parser, Subcommand};

const EXAMPLES: &str = "\x1b[1mExamples:\x1b[0m
  whypkg                     Browse every installed package
  whypkg --upgradable        Browse only packages with a pending upgrade
  whypkg pending             Report every pending upgrade, grouped by why it's here
  whypkg pending --quick     One line per pending package: size + reason
  whypkg update              Update whypkg to the latest release

Inside the browser: type to filter, Enter to open, Esc to go back.";

#[derive(Parser)]
#[command(
    name = "whypkg",
    bin_name = "whypkg",
    version,
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
    /// Update whypkg itself to the latest release (cargo install whypkg --force)
    Update(commands::update::Args),
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
        Some(Cmd::Update(args)) => commands::update::run(args),
        // No subcommand → the interactive browser (the heart of the tool).
        None => commands::browse::run(commands::browse::Args {
            upgradable: cli.upgradable,
        }),
    }
}
