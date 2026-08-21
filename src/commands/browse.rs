//! `whypkg` (default): the interactive investigator, which is the soul of the
//! tool. This file only builds the world and hands it to the interface in
//! `crate::tui`.

use crate::model::World;
use crate::tui::filter::FilterMode;
use crate::tui::{App, Frame, Origin, Relation};
use nucleo_matcher::{Config, Matcher};

pub struct Args {
    pub upgradable: bool,
}

pub fn run(args: Args) {
    let world = load_world_with_notice();

    let pool = if args.upgradable {
        world.upgradable_names_sorted()
    } else {
        world.all_names_sorted()
    };

    if pool.is_empty() {
        if args.upgradable {
            println!("\n  Nothing to upgrade - system is up to date.\n");
        } else {
            println!("\n  No packages found.\n");
        }
        return;
    }

    let mut app = App {
        world,
        matcher: Matcher::new(Config::DEFAULT),
        filter: FilterMode::All,
        relation: Relation::NeededBy,
        graph: None,
        stack: vec![Frame {
            focus: None,
            pool,
            needed_by: Vec::new(),
            depends_on: Vec::new(),
            query: String::new(),
            selected: 0,
            alongside: Vec::new(),
            origin: Origin::None,
        }],
    };

    if let Err(e) = app.run_ui() {
        eprintln!("whypkg: terminal error: {e}");
        std::process::exit(1);
    }
}

/// Load the world, but since this can take ~½ s on a big system, print a one
/// line notice first so the user isn't staring at a blank terminal.
pub(super) fn load_world_with_notice() -> World {
    eprint!("  loading package data…\r");
    let world = crate::commands::load_world();
    eprint!("                        \r");
    world
}
