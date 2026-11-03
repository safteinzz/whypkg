//! `whypkg leaves`: the installed packages worth a second look, grouped by why.
//! It only lists them; removing any of them is the package manager's job.

use crate::commands::load_world;
use crate::engine::{count, format_size};
use crate::model::{Origin, World};
use colored::Colorize;
use serde_json::json;
use std::cmp::Reverse;
use std::collections::HashSet;

/// What it prints, on `whypkg leaves --help`.
const FIELDS: &str = "\
Reasons: `unneeded` (the package manager's autoremove would take it), `local`
(installed from a file) and `orphaned` (no enabled repository offers it). A package
gets the first that fits. Flatpak runtimes are never `unneeded`, since flatpak has
no read-only way to ask.

--json fields: name, version, description, source (`system` or `flatpak`),
installed (YYYY-MM-DD or null), size_kb and reason.";

#[derive(clap::Args)]
#[command(after_help = FIELDS)]
pub struct Args {
    /// One line per package: size + reason, nothing else
    #[arg(long, conflicts_with = "json")]
    pub quick: bool,
    /// Print a JSON array, one object per package, instead of text
    #[arg(long)]
    pub json: bool,
}

/// Why a package is listed. Each package gets the first that fits, in this
/// order, so the groups never overlap.
#[derive(Clone, Copy, PartialEq)]
enum Leaf {
    /// The package manager's autoremove would take it.
    Unneeded,
    /// Installed from a local file rather than a repository.
    Local,
    /// No configured repository offers it, so it gets no updates.
    Orphaned,
}

impl Leaf {
    fn word(self) -> &'static str {
        match self {
            Leaf::Unneeded => "unneeded",
            Leaf::Local => "local",
            Leaf::Orphaned => "orphaned",
        }
    }
}

fn classify(world: &World, unneeded: &HashSet<String>, pkg: &str) -> Option<Leaf> {
    if unneeded.contains(pkg) {
        return Some(Leaf::Unneeded);
    }
    match world.packages[pkg].origin {
        Origin::Local => Some(Leaf::Local),
        Origin::Orphaned => Some(Leaf::Orphaned),
        _ => None,
    }
}

/// Ask the system package manager for its autoremove list. Flatpak has no
/// read-only way to ask, so its runtimes are never listed as unneeded.
fn query_unneeded() -> HashSet<String> {
    let Some(backend) = crate::backend::detect() else {
        return HashSet::new();
    };
    match backend.unneeded() {
        Ok(names) => names.into_iter().collect(),
        Err(e) => {
            eprintln!(
                "whypkg: could not ask {} what is unneeded: {e}",
                backend.name()
            );
            std::process::exit(1);
        }
    }
}

pub fn run(args: Args) {
    let world = load_world();
    let unneeded = query_unneeded();
    let leaves: Vec<(String, Leaf)> = world
        .all_names_sorted()
        .into_iter()
        .filter_map(|pkg| classify(&world, &unneeded, &pkg).map(|leaf| (pkg, leaf)))
        .collect();

    if args.json {
        let out: Vec<serde_json::Value> = leaves
            .iter()
            .map(|(pkg, leaf)| {
                let p = &world.packages[pkg];
                json!({
                    "name": pkg,
                    "version": p.version,
                    "description": p.description,
                    "source": p.source.word(),
                    "installed": p.install_date,
                    "size_kb": p.installed_size,
                    "reason": leaf.word(),
                })
            })
            .collect();
        println!("{}", serde_json::Value::Array(out));
        return;
    }

    if args.quick {
        let mut by_size: Vec<&(String, Leaf)> = leaves.iter().collect();
        by_size.sort_by_key(|(pkg, _)| Reverse(world.packages[pkg].installed_size));
        for (pkg, leaf) in by_size {
            println!(
                "{:<36}  {:>10}   {}",
                pkg,
                format_size(world.packages[pkg].installed_size),
                leaf.word()
            );
        }
        return;
    }

    if leaves.is_empty() {
        println!(
            "\n  {}\n",
            "Nothing to look at: everything is needed and from a repo.".green()
        );
        return;
    }

    println!(
        "\n{}  {}",
        "whypkg leaves".bold().cyan(),
        format!("{} worth a look", count(leaves.len(), "package")).dimmed()
    );
    section(
        &world,
        &leaves,
        Leaf::Unneeded,
        "NOTHING NEEDS THESE",
        "Auto-installed and needed by nothing: autoremove would take them.",
    );
    section(
        &world,
        &leaves,
        Leaf::Local,
        "INSTALLED FROM A LOCAL FILE",
        "Sideloaded rather than installed from a repository.",
    );
    section(
        &world,
        &leaves,
        Leaf::Orphaned,
        "NOT IN ANY REPO",
        "No configured repository offers these, so they get no updates.",
    );
    println!(
        "\n{}\n{}\n",
        "  whypkg why <pkg>        everything about one of them".dimmed(),
        "  whypkg leaves --quick   one line per package".dimmed(),
    );
}

fn section(world: &World, leaves: &[(String, Leaf)], which: Leaf, title: &str, note: &str) {
    let list: Vec<&String> = leaves
        .iter()
        .filter(|(_, leaf)| *leaf == which)
        .map(|(pkg, _)| pkg)
        .collect();
    if list.is_empty() {
        return;
    }
    let total: u64 = list.iter().map(|p| world.packages[*p].installed_size).sum();
    println!(
        "\n{}",
        format!(
            "  {title}  ({}, {})",
            count(list.len(), "package"),
            format_size(total)
        )
        .bold()
        .white()
    );
    println!("  {}", "─".repeat(58).dimmed());
    println!("\n  {}", note.dimmed());
    for pkg in list {
        let p = &world.packages[pkg];
        println!(
            "\n  {}  {}",
            format!("{pkg:<32}").yellow(),
            format_size(p.installed_size)
        );
        println!("  {}", p.description.dimmed());
    }
}
