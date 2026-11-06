//! whypkg - why the hell is this package here?
//!
//! A fast, cross-distro package investigator. The interactive browser and the
//! `why`, `pending` and `leaves` reports share one engine.
//!
//! This file is the clap `Cmd` enum and the dispatch match; what you can run is
//! `whypkg --help`, which renders from the manifest, those doc comments and
//! `AFTER`, and is the only copy of that list.
//!
//! The package-manager specifics live behind a single `Backend` trait, so apt,
//! pacman, dnf and zypper all feed the same analysis and the same UI, and the
//! installers beside it (flatpak, cargo, pipx, uv, npm) each add a layer on top.

mod backend;
mod commands;
mod config;
mod engine;
mod load;
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
  whypkg    browse every installed package, a tab per installer (TUI)
              enter opens a package, / filters, ? lists every key, q quits";

/// The rest of the block: the words the output prints, what a script can
/// expect, then where to look next.
const AFTER: &str = concat!(
    "\
Sources: `system` is the distro's package manager; `flatpak`, `cargo`, `pipx`, `uv`
and `npm` sit beside it, with what a tool pulled into its own virtualenv or
node_modules. `--only` picks some for one run, and `skip` in
~/.config/whypkg/config.toml (`o` in the browser) leaves some out for good.

Reasons: `you` installed it, `pulled-in` by something you installed (`← pkg` in
`pending --quick`), or `auto` with nothing you installed needing it. `pending` adds
`kernel` and `system lib` (`system-lib` in --json) for what more than 25 packages
need, and `leaves` prints `unneeded`, `local` and `orphaned` instead.

`--quick` prints one line per package and `--json` one array of objects, both on
stdout and ending quietly when the reader goes away; the JSON field names and words
are fixed and listed in each command's --help. Every other output is rendered for
people, and the browser takes over the terminal. Nothing prompts, and nothing on
the system is changed. Failures go to stderr and exit non-zero; `why` exits 1 when a
package is not installed, after printing the rest.
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
    // `--only` and `--upgradable` belong to the browser; each command takes its
    // own, so one typed before a command is refused rather than ignored.
    args_conflicts_with_subcommands = true,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,

    /// Limit the browser to packages with a pending upgrade
    #[arg(long)]
    upgradable: bool,

    /// Read only these sources, instead of every one the settings leave on
    #[arg(long, value_delimiter = ',', value_name = "SOURCES", value_parser = crate::commands::source_parser())]
    only: Vec<model::Source>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Explain why packages are installed, without the browser  <PKG>...
    ///   --json            one JSON object per package
    ///   --only SOURCES    read only these, comma-separated
    #[command(verbatim_doc_comment)]
    Why(commands::why::Args),
    /// Report every pending upgrade, grouped by why it's on your system
    ///   --quick           one line per package: size + reason
    ///   --kernel          kernel, firmware and microcode only
    ///   --apps            only the packages you installed yourself
    #[command(verbatim_doc_comment)]
    Pending(commands::pending::Args),
    /// List packages worth a second look: unneeded, sideloaded and orphaned
    ///   --quick           one line per package: size + reason
    ///   --json            one JSON object per package
    ///   --only SOURCES    read only these, comma-separated
    #[command(verbatim_doc_comment)]
    Leaves(commands::leaves::Args),
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
        Some(Cmd::Why(args)) => commands::why::run(args),
        Some(Cmd::Pending(args)) => commands::pending::run(args),
        Some(Cmd::Leaves(args)) => commands::leaves::run(args),
        Some(Cmd::Selfie(cmd)) => commands::selfcmd::run(cmd),
        // No subcommand → the interactive browser (the heart of the tool).
        None => commands::browse::run(commands::browse::Args {
            upgradable: cli.upgradable,
            only: cli.only,
        }),
    }
}
