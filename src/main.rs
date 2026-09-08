//! whypkg - why the hell is this package here?
//!
//! A fast, cross-distro package investigator. Two modes share one engine: an
//! interactive browser and a grouped report.
//!
//! This file is the clap `Cmd` enum and the dispatch match; what you can run is
//! `whypkg --help`, which renders from the manifest, those doc comments and
//! `AFTER`, and is the only copy of that list.
//!
//! The package-manager specifics live behind a single `Backend` trait, so apt,
//! pacman, and dnf all feed the same analysis and the same UI.

mod backend;
mod commands;
mod engine;
mod model;
mod tui;

use clap::{Parser, Subcommand};

/// clap's own layout with one change: `{before-help}` moves from above the
/// description to just under `Usage:`, so the shapes block lands on top of the
/// command list rather than on top of the screen.
const TEMPLATE: &str =
    "{about-with-newline}\n{usage-heading} {usage}\n\n{before-help}{all-args}{after-help}\n";

/// The shapes clap cannot list, because the browser is the bare invocation
/// rather than a subcommand.
const WAYS: &str = "\x1b[1mWays to run it (not subcommands):\x1b[0m
  whypkg    browse every installed package (TUI)
              type to filter, enter opens a dossier, esc goes back";

/// The rest of the block: what a script can expect, then where to look next.
const AFTER: &str = concat!(
    "\
`pending --quick` prints one line per package for a pipe, and ends quietly when
the reader goes away; every other output is rendered for people, and the browser
takes over the terminal. Failures name themselves on stderr and exit non-zero.
Run `whypkg <command> --help` for a command's details.",
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
    // The shapes come first: this is a bare-first binary, so the command list is
    // the leftovers and putting it on top answers the wrong question first.
    help_template = TEMPLATE,
    before_help = WAYS,
    after_help = AFTER,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,

    /// Limit the browser to packages with a pending upgrade
    #[arg(long)]
    upgradable: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Report every pending upgrade, grouped by why it's on your system
    ///   --quick   one line per package: size + reason
    ///   --kernel  kernel, firmware and microcode only
    ///   --apps    only the packages you installed yourself
    #[command(verbatim_doc_comment)]
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
