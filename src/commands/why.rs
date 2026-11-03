//! `whypkg why <PKG>...`: a package's dossier printed instead of browsed, so a
//! script, an agent or a pipe gets the same answer the browser gives.

use crate::commands::load_world;
use crate::engine::{Why, format_size, is_kernel_pkg, relative_time, same_session, why};
use crate::model::{Origin, Source, World};
use colored::Colorize;
use serde_json::json;

/// What `--json` holds, on `whypkg why --help`.
const FIELDS: &str = "\
--json fields, one object per installed package: name, version, candidate (the
upgrade waiting, or null), description, source (`system` or `flatpak`), remote,
origin (`repo`, `local`, `orphaned` or `unknown`), installed (YYYY-MM-DD or null),
size_kb, reason (`you`, `pulled-in` or `auto`), chain (from what you installed down
to this package, empty unless pulled-in), kernel (true for kernel and firmware),
needed_by, depends_on and alongside (installed within the same few hours).";

#[derive(clap::Args)]
#[command(after_help = FIELDS)]
pub struct Args {
    /// Installed packages to explain, spelled as the package manager lists them
    #[arg(required = true, value_name = "PKG")]
    pub packages: Vec<String>,
    /// Print a JSON array, one object per package, instead of text
    #[arg(long)]
    pub json: bool,
}

/// How many names a text line lists before it says `+N more`.
const PREVIEW: usize = 5;

pub fn run(args: Args) {
    let world = load_world();

    let mut found = Vec::new();
    let mut missing = false;
    for name in &args.packages {
        if world.packages.contains_key(name) {
            found.push(name.as_str());
        } else {
            missing = true;
            not_installed(&world, name);
        }
    }

    if args.json {
        let out: Vec<serde_json::Value> = found.iter().map(|p| to_json(&world, p)).collect();
        println!("{}", serde_json::Value::Array(out));
    } else {
        for (i, pkg) in found.iter().enumerate() {
            if i > 0 {
                println!();
            }
            print_text(&world, pkg);
        }
    }

    if missing {
        std::process::exit(1);
    }
}

/// Say on stderr that `name` is not installed, with up to three installed
/// packages whose name or description contains it, since the name a person
/// knows (`element`) is often not the one the package manager uses
/// (`im.riot.Riot`).
fn not_installed(world: &World, name: &str) {
    let needle = name.to_lowercase();
    let mut by_name = Vec::new();
    let mut by_desc = Vec::new();
    for pkg in world.all_names_sorted() {
        if pkg.to_lowercase().contains(&needle) {
            by_name.push(pkg);
        } else if world.packages[&pkg]
            .description
            .to_lowercase()
            .contains(&needle)
        {
            by_desc.push(pkg);
        }
    }
    by_name.sort_by_key(String::len);
    let close: Vec<String> = by_name
        .into_iter()
        .chain(by_desc)
        .take(3)
        .map(|p| format!("`{p}`"))
        .collect();
    if close.is_empty() {
        eprintln!(
            "whypkg: `{name}` is not installed, and no installed package mentions it: check the spelling"
        );
    } else {
        eprintln!(
            "whypkg: `{name}` is not installed, did you mean {}?",
            close.join(", ")
        );
    }
}

fn sorted(names: &[String]) -> Vec<String> {
    let mut out = names.to_vec();
    out.sort();
    out.dedup();
    out
}

fn preview(names: &[String]) -> String {
    let mut text = names
        .iter()
        .take(PREVIEW)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > PREVIEW {
        text.push_str(&format!(", +{} more", names.len() - PREVIEW));
    }
    text
}

fn print_text(world: &World, pkg: &str) {
    let p = &world.packages[pkg];
    let kv = |k: &str, v: String| println!("  {}{v}", format!("{k:<12}").dimmed());

    let upgrade = match &p.candidate {
        Some(c) => format!("  ↑ {c}").cyan().to_string(),
        None => String::new(),
    };
    println!("{}  {}{upgrade}", pkg.bold(), p.version.dimmed());
    if !p.description.is_empty() {
        println!("  {}", p.description.dimmed());
    }

    match why(world, pkg) {
        Why::You => kv("why here", "you installed this".green().to_string()),
        Why::PulledIn(chain) => {
            let root = chain.first().cloned().unwrap_or_default();
            kv(
                "why here",
                format!("pulled in by {}", root.bold()).yellow().to_string(),
            );
            kv("chain", chain.join(" → "));
        }
        Why::Untraced => kv(
            "why here",
            "auto-installed, and nothing you installed needs it"
                .yellow()
                .to_string(),
        ),
    }

    let alongside = same_session(world, pkg);
    if !alongside.is_empty() {
        kv("alongside", preview(&alongside));
    }
    let installed = match (&p.install_date, p.install_epoch) {
        (Some(date), Some(epoch)) => format!("{date} ({})", relative_time(epoch)),
        (Some(date), None) => date.clone(),
        _ => "unknown".to_string(),
    };
    kv("installed", installed);
    kv("size", format_size(p.installed_size));

    let source = match (p.source, p.origin) {
        (Source::Flatpak, _) => Some(match &p.remote {
            Some(remote) => format!("flatpak ({remote})").blue().to_string(),
            None => "flatpak app".blue().to_string(),
        }),
        (_, Origin::Repo) => Some("from a repository".green().to_string()),
        (_, Origin::Local) => Some("installed from a local file".yellow().to_string()),
        (_, Origin::Orphaned) => Some("not in any repo (repo removed?)".red().to_string()),
        (_, Origin::Unknown) => None,
    };
    if let Some(source) = source {
        kv("source", source);
    }

    let needed_by = sorted(world.rdeps_of(pkg));
    let needed_text = if !needed_by.is_empty() {
        format!("{}: {}", needed_by.len(), preview(&needed_by))
    } else if is_kernel_pkg(pkg) {
        "nothing, but it is kernel or firmware".to_string()
    } else {
        "nothing".to_string()
    };
    kv("needed by", needed_text);

    let depends_on = sorted(world.deps_of(pkg));
    let depends_text = if depends_on.is_empty() {
        "nothing".to_string()
    } else {
        format!("{}: {}", depends_on.len(), preview(&depends_on))
    };
    kv("depends on", depends_text);
}

fn to_json(world: &World, pkg: &str) -> serde_json::Value {
    let p = &world.packages[pkg];
    let (reason, chain) = match why(world, pkg) {
        Why::You => ("you", Vec::new()),
        Why::PulledIn(chain) => ("pulled-in", chain),
        Why::Untraced => ("auto", Vec::new()),
    };
    json!({
        "name": pkg,
        "version": p.version,
        "candidate": p.candidate,
        "description": p.description,
        "source": p.source.word(),
        "remote": p.remote,
        "origin": p.origin.word(),
        "installed": p.install_date,
        "size_kb": p.installed_size,
        "reason": reason,
        "chain": chain,
        "kernel": is_kernel_pkg(pkg),
        "needed_by": sorted(world.rdeps_of(pkg)),
        "depends_on": sorted(world.deps_of(pkg)),
        "alongside": same_session(world, pkg),
    })
}
