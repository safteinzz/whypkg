//! `whypkg why <PKG>...`: a package's dossier printed instead of browsed, so a
//! script, an agent or a pipe gets the same answer the browser gives.

use crate::commands::{load_world, sources};
use crate::engine::{Why, format_size, is_kernel_pkg, relative_time, same_session, why};
use crate::model::{Origin, Source, World};
use colored::Colorize;
use serde_json::json;

/// What `--json` holds, on `whypkg why --help`.
const FIELDS: &str = "\
A PKG is a name as its installer spells it (`ripgrep`), which explains every
source's copy, or a key from --json (`cargo:ripgrep`), which explains that one.

--json fields, one object per package: name (the key, unique across sources),
label (the name a person knows it by), version, candidate (the upgrade waiting, or
null), description, source (`system`, `flatpak`, `cargo`, `pipx`, `uv` or `npm`),
remote, origin (`repo`, `local`, `orphaned` or `unknown`), installed (YYYY-MM-DD or
null), size_kb, reason (`you`, `pulled-in` or `auto`), chain (keys from what you
installed down to this package, empty unless pulled-in), kernel (true for kernel
and firmware), needed_by, depends_on, uses (system packages it loads or runs on),
used_by (packages from other installers that load or run on it), remove (the
command that would uninstall it, which whypkg never runs, or null) and alongside
(installed within the same few hours).";

#[derive(clap::Args)]
#[command(after_help = FIELDS)]
pub struct Args {
    /// Installed packages to explain
    #[arg(required = true, value_name = "PKG")]
    pub packages: Vec<String>,
    /// Print a JSON array, one object per package, instead of text
    #[arg(long)]
    pub json: bool,
    /// Read only these sources, instead of every one the settings leave on
    #[arg(long, value_delimiter = ',', value_name = "SOURCES", value_parser = crate::commands::source_parser())]
    pub only: Vec<Source>,
}

/// How many names a text line lists before it says `+N more`.
const PREVIEW: usize = 5;

pub fn run(args: Args) {
    let world = load_world(&sources(&args.only));

    let mut found = Vec::new();
    let mut missing = false;
    for name in &args.packages {
        let keys = lookup(&world, name);
        if keys.is_empty() {
            missing = true;
            not_installed(&world, name);
        }
        for key in keys {
            if !found.contains(&key) {
                found.push(key);
            }
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

/// The keys `name` means: every package of that name, in the order the
/// sources are listed, or else the one package whose key it is. A pipx or uv
/// name matches the way pip compares them (`pygments` is `Pygments`).
fn lookup(world: &World, name: &str) -> Vec<String> {
    let pip_name = crate::backend::venv::normalize(name);
    let mut keys: Vec<&crate::model::Package> = world
        .packages
        .values()
        .filter(|p| {
            p.label() == name
                || (matches!(p.source, Source::Pipx | Source::Uv)
                    && crate::backend::venv::normalize(p.label()) == pip_name)
        })
        .collect();
    if keys.is_empty() && world.packages.contains_key(name) {
        return vec![name.to_string()];
    }
    keys.sort_by_key(|p| {
        (
            Source::ALL.iter().position(|s| *s == p.source),
            p.name.clone(),
        )
    });
    keys.into_iter().map(|p| p.name.clone()).collect()
}

/// Say on stderr that `name` is not installed, with up to three installed
/// packages whose name or description contains it, since the name a person
/// knows (`element`) is often not the one the package manager uses
/// (`im.riot.Riot`).
fn not_installed(world: &World, name: &str) {
    let needle = name.to_lowercase();
    let mut by_name = Vec::new();
    let mut by_desc = Vec::new();
    for key in world.all_names_sorted() {
        let p = &world.packages[&key];
        if p.label().to_lowercase().contains(&needle) {
            by_name.push(key);
        } else if p.description.to_lowercase().contains(&needle) {
            by_desc.push(key);
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

/// Names for people: each key's label, with its source when that is not the
/// system's (`ripgrep (cargo)`).
fn labelled(world: &World, keys: &[String]) -> Vec<String> {
    keys.iter()
        .map(|k| match world.source(k) {
            Source::System => world.label(k).to_string(),
            source => format!("{} ({})", world.label(k), source.word()),
        })
        .collect()
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

/// Where a package came from, in words, or `None` when the backend could not
/// tell.
pub fn source_text(world: &World, pkg: &str) -> Option<String> {
    let p = &world.packages[pkg];
    let remote = p.remote.as_deref().unwrap_or_default();
    let installer = match p.source {
        Source::System => {
            return match p.origin {
                Origin::Repo => Some("from a repository".to_string()),
                Origin::Local => Some("installed from a local file".to_string()),
                Origin::Orphaned => Some("not in any repo (repo removed?)".to_string()),
                Origin::Unknown => None,
            };
        }
        Source::Flatpak if remote.is_empty() => return Some("flatpak app".to_string()),
        Source::Flatpak => return Some(format!("flatpak ({remote})")),
        Source::Cargo => "cargo install",
        Source::Pipx => "pipx",
        Source::Uv => "uv tool",
        Source::Npm => "npm -g",
    };
    Some(match p.origin {
        Origin::Local => format!("{installer}, from {remote}"),
        _ if remote.is_empty() => installer.to_string(),
        _ => format!("{installer}, from {remote}"),
    })
}

fn print_text(world: &World, pkg: &str) {
    let p = &world.packages[pkg];
    let kv = |k: &str, v: String| println!("  {}{v}", format!("{k:<12}").dimmed());

    let upgrade = match &p.candidate {
        Some(c) => format!("  ↑ {c}").cyan().to_string(),
        None => String::new(),
    };
    let key = if p.label() == pkg {
        String::new()
    } else {
        format!("  {}", pkg.dimmed())
    };
    println!("{}  {}{upgrade}{key}", p.label().bold(), p.version.dimmed());
    if !p.description.is_empty() {
        println!("  {}", p.description.dimmed());
    }

    match why(world, pkg) {
        Why::You => kv("why here", "you installed this".green().to_string()),
        Why::PulledIn(chain) => {
            let root = chain.first().map(|r| world.label(r)).unwrap_or_default();
            kv(
                "why here",
                format!("pulled in by {}", root.bold()).yellow().to_string(),
            );
            let chain: Vec<&str> = chain.iter().map(|k| world.label(k)).collect();
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
    if let Some(source) = source_text(world, pkg) {
        let coloured = match (p.source, p.origin) {
            (Source::System, Origin::Repo) => source.green(),
            (Source::System, Origin::Orphaned) => source.red(),
            (_, Origin::Local) => source.yellow(),
            _ => source.blue(),
        };
        kv("source", coloured.to_string());
    }

    let needed_by = sorted(world.rdeps_of(pkg));
    let used_by = sorted(world.used_by_of(pkg));
    let needed_text = if !needed_by.is_empty() {
        format!(
            "{}: {}",
            needed_by.len(),
            preview(&labelled(world, &needed_by))
        )
    } else if is_kernel_pkg(pkg) {
        "nothing, but it is kernel or firmware".to_string()
    } else {
        "nothing".to_string()
    };
    kv("needed by", needed_text);
    if !used_by.is_empty() {
        kv(
            "used by",
            format!("{}: {}", used_by.len(), preview(&labelled(world, &used_by)))
                .yellow()
                .to_string(),
        );
    }

    let depends_on = sorted(world.deps_of(pkg));
    let depends_text = if depends_on.is_empty() {
        "nothing".to_string()
    } else {
        format!(
            "{}: {}",
            depends_on.len(),
            preview(&labelled(world, &depends_on))
        )
    };
    kv("depends on", depends_text);
    let uses = sorted(world.uses_of(pkg));
    if !uses.is_empty() {
        kv(
            "uses",
            format!("{}: {}", uses.len(), preview(&labelled(world, &uses))),
        );
    }
    // Only offered when nothing needs it, so the line never reads as advice
    // to take out something the system stands on.
    if needed_by.is_empty()
        && !is_kernel_pkg(pkg)
        && let Some(cmd) = world.remove_command(pkg)
    {
        kv("remove", cmd.dimmed().to_string());
    }
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
        "label": p.label(),
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
        "uses": sorted(world.uses_of(pkg)),
        "used_by": sorted(world.used_by_of(pkg)),
        "remove": world.remove_command(pkg),
        "alongside": same_session(world, pkg),
    })
}
