//! `whypkg` (default): the interactive investigator, which is the soul of the
//! tool. This file only reads the machine and hands it to the interface in
//! `crate::tui`.

use crate::commands::sources;
use crate::load::read_world;
use crate::model::Source;
use crate::tui::{App, Show, while_loading};
use std::io::IsTerminal;

pub struct Args {
    pub upgradable: bool,
    pub only: Vec<Source>,
}

pub fn run(args: Args) {
    // Checked before the machine is read, so a pipe fails at once.
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!(
            "whypkg: the browser needs a terminal: in a script or a pipe, run `whypkg why <PKG>`, `whypkg leaves` or `whypkg pending`"
        );
        std::process::exit(1);
    }
    let sources = sources(&args.only);
    let world = match while_loading(|progress| read_world(&sources, progress)) {
        Ok(world) => world,
        Err(e) => {
            eprintln!("whypkg: {e}");
            std::process::exit(1);
        }
    };

    if world.packages.is_empty() {
        println!("\n  No packages found.\n");
        return;
    }
    if args.upgradable && !sources.has(Source::System) {
        println!(
            "\n  Nothing to show: only the system's packages report upgrades, and --only leaves them out.\n"
        );
        return;
    }
    if args.upgradable && world.upgradable_names_sorted().is_empty() {
        println!("\n  Nothing to upgrade: the system is up to date.\n");
        return;
    }

    let show = if args.upgradable {
        Show::Upgradable
    } else {
        Show::All
    };
    let mut app = App::new(world, sources, show);
    if let Err(e) = app.run_ui() {
        eprintln!("whypkg: the terminal stopped answering ({})", e.kind());
        std::process::exit(1);
    }
}
