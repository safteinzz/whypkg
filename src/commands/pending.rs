//! `whypkg pending` — understand every package with a pending upgrade, grouped
//! by *why* it's on your system. Run after a package-list refresh to know what
//! you're about to pull down and which of it you actually chose.
//!
//! This is the port of the old `apt-pending` script. Each upgradable package is
//! classified once — kernel/firmware, foundational system library, an app you
//! installed, or an auto-installed package traced back (via reverse-dep BFS) to
//! the manual package that pulled it in.

use crate::commands::load_world;
use crate::engine::{FOUNDATION_THRESHOLD, bfs_root, format_size, is_kernel_pkg};
use crate::model::World;
use colored::Colorize;
use std::cmp::Reverse;
use std::collections::HashMap;

#[derive(clap::Args)]
pub struct Args {
    /// Kernel, firmware and microcode updates only
    #[arg(long)]
    pub kernel: bool,
    /// Packages you explicitly installed
    #[arg(long)]
    pub apps: bool,
    /// Auto-installed packages, grouped by what pulled them in
    #[arg(long)]
    pub auto: bool,
    /// One line per package: size + reason, nothing else
    #[arg(long)]
    pub quick: bool,
    /// Top packages by installed size
    #[arg(long)]
    pub sizes: bool,
}

/// How an upgradable package is categorised (drives display + grouping).
enum Category {
    Kernel,
    Foundation,
    App,
    /// Auto-installed, traced to the manual package that pulled it in.
    PulledIn(String),
    /// Auto-installed but no manual ancestor found.
    Untraced,
}

pub fn run(args: Args) {
    let world = load_world();

    let upgradable = world.upgradable_names_sorted();
    if upgradable.is_empty() {
        println!("\n  {}\n", "System is fully up to date.".green());
        return;
    }

    // Classify every upgradable package exactly once.
    let mut category: HashMap<String, Category> = HashMap::new();
    for pkg in &upgradable {
        category.insert(pkg.clone(), classify(&world, pkg));
    }

    // With no flags, show the full report.
    let show_all = !(args.kernel || args.apps || args.auto || args.quick || args.sizes);

    if args.quick {
        section_quick(&world, &upgradable, &category);
        return;
    }

    println!(
        "\n{}  {}",
        "whypkg pending".bold().cyan(),
        format!("{} packages to upgrade", upgradable.len()).dimmed()
    );

    if show_all || args.kernel {
        section_kernel(&world, &upgradable, &category);
    }
    if show_all || args.apps {
        section_apps(&world, &upgradable, &category);
    }
    if show_all || args.auto {
        section_auto(&world, &upgradable, &category);
    }
    if show_all || args.sizes {
        section_sizes(&world, &upgradable, &category);
    }

    println!(
        "\n{}\n{}\n{}\n",
        "  upgrade everything       see your package manager (apt upgrade, …)".dimmed(),
        "  whypkg --upgradable      browse these interactively".dimmed(),
        "  whypkg pending --quick   one line per package".dimmed(),
    );
}

/// Decide a package's category. Priority mirrors the original tool:
/// kernel > foundational lib > (auto traced to a manual root) > your app > untraced.
fn classify(world: &World, pkg: &str) -> Category {
    if is_kernel_pkg(pkg) {
        return Category::Kernel;
    }
    if world.rdep_count(pkg) > FOUNDATION_THRESHOLD {
        return Category::Foundation;
    }
    if world.is_manual(pkg) {
        return Category::App;
    }
    match bfs_root(world, pkg) {
        Some(path) => Category::PulledIn(path.last().cloned().unwrap_or_default()),
        None => Category::Untraced,
    }
}

fn header(title: &str) {
    println!("\n{}", format!("  {title}").bold().white());
    println!("  {}", "─".repeat(58).dimmed());
}

fn version_change(world: &World, pkg: &str) -> String {
    let p = match world.packages.get(pkg) {
        Some(p) => p,
        None => return String::new(),
    };
    match &p.candidate {
        Some(c) => format!("{}  →  {}", p.version, c),
        None => p.version.clone(),
    }
}

fn desc(world: &World, pkg: &str) -> String {
    world
        .packages
        .get(pkg)
        .map(|p| p.description.clone())
        .unwrap_or_default()
}

fn size_kb(world: &World, pkg: &str) -> u64 {
    world
        .packages
        .get(pkg)
        .map(|p| p.installed_size)
        .unwrap_or(0)
}

// ── sections ────────────────────────────────────────────────────────────────

fn section_kernel(world: &World, pool: &[String], cat: &HashMap<String, Category>) {
    let list: Vec<&String> = pool
        .iter()
        .filter(|p| {
            matches!(
                cat.get(*p),
                Some(Category::Kernel) | Some(Category::Foundation)
            )
        })
        .collect();
    if list.is_empty() {
        return;
    }
    header(&format!(
        "KERNEL, FIRMWARE & SYSTEM LIBS  ({} packages)",
        list.len()
    ));
    for pkg in list {
        let note = match cat.get(pkg) {
            Some(Category::Foundation) => {
                format!("  ({} packages depend on this)", world.rdep_count(pkg))
                    .dimmed()
                    .to_string()
            }
            _ => String::new(),
        };
        println!(
            "\n  {}  {}{}",
            format!("{pkg:<32}").bold(),
            format_size(size_kb(world, pkg)),
            note
        );
        println!("  {}", desc(world, pkg).dimmed());
        println!("  {}", version_change(world, pkg).dimmed());
    }
}

fn section_apps(world: &World, pool: &[String], cat: &HashMap<String, Category>) {
    let list: Vec<&String> = pool
        .iter()
        .filter(|p| matches!(cat.get(*p), Some(Category::App)))
        .collect();
    if list.is_empty() {
        return;
    }
    header(&format!("YOUR INSTALLED APPS  ({} packages)", list.len()));
    println!("\n  {}", "Packages you explicitly installed.".dimmed());
    for pkg in list {
        println!(
            "\n  {} {}  {}",
            ">>".bold().green(),
            format!("{pkg:<28}").bold().green(),
            format_size(size_kb(world, pkg))
        );
        println!("  {}", desc(world, pkg).dimmed());
        println!("  {}", version_change(world, pkg).dimmed());
    }
}

fn section_auto(world: &World, pool: &[String], cat: &HashMap<String, Category>) {
    // Group pulled-in packages by the manual root that caused them.
    let mut groups: HashMap<String, Vec<&String>> = HashMap::new();
    let mut untraced: Vec<&String> = Vec::new();
    for pkg in pool {
        match cat.get(pkg) {
            Some(Category::PulledIn(root)) => groups.entry(root.clone()).or_default().push(pkg),
            Some(Category::Untraced) => untraced.push(pkg),
            _ => {}
        }
    }

    // Largest groups first — they're the most worth understanding.
    let mut roots: Vec<(&String, &Vec<&String>)> = groups.iter().collect();
    roots.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));

    for (root, members) in roots {
        header(&format!(
            "PULLED IN BY: {root}  ({} packages)",
            members.len()
        ));
        println!("  {}", desc(world, root).green());
        println!(
            "  {}",
            format!(
                "All {} packages below exist because you installed {root}.",
                members.len()
            )
            .dimmed()
        );
        let mut sorted = members.clone();
        sorted.sort();
        for pkg in sorted {
            println!(
                "\n  {}  {}",
                format!("{pkg:<32}").yellow(),
                format_size(size_kb(world, pkg))
            );
            println!("  {}", desc(world, pkg).dimmed());
        }
    }

    if !untraced.is_empty() {
        header(&format!(
            "OTHER AUTO-INSTALLED  ({} packages)",
            untraced.len()
        ));
        println!(
            "\n  {}",
            "Auto-installed, but no manual package found in the dependency chain.".dimmed()
        );
        untraced.sort();
        for pkg in untraced {
            println!(
                "\n  {}  {}",
                format!("{pkg:<32}").yellow(),
                format_size(size_kb(world, pkg))
            );
            println!("  {}", desc(world, pkg).dimmed());
        }
    }
}

fn section_sizes(world: &World, pool: &[String], cat: &HashMap<String, Category>) {
    header("DISK USAGE  (top 20 by installed size)");
    let mut by_size: Vec<&String> = pool.iter().collect();
    by_size.sort_by_key(|p| Reverse(size_kb(world, p)));
    println!();
    for pkg in by_size.iter().take(20) {
        println!(
            "  {:<34}  {:>10}   {}",
            pkg,
            format_size(size_kb(world, pkg)),
            reason(cat.get(*pkg)).dimmed()
        );
    }
    let total: u64 = pool.iter().map(|p| size_kb(world, p)).sum();
    println!(
        "\n  {}",
        format!(
            "Total: {} across {} packages",
            format_size(total),
            pool.len()
        )
        .bold()
    );
}

fn section_quick(world: &World, pool: &[String], cat: &HashMap<String, Category>) {
    let mut by_size: Vec<&String> = pool.iter().collect();
    by_size.sort_by_key(|p| Reverse(size_kb(world, p)));
    for pkg in by_size {
        println!(
            "{:<36}  {:>10}   {}",
            pkg,
            format_size(size_kb(world, pkg)),
            reason(cat.get(pkg))
        );
    }
}

/// One-word "why is it here" reason for the compact views.
fn reason(cat: Option<&Category>) -> String {
    match cat {
        Some(Category::Kernel) => "kernel".to_string(),
        Some(Category::Foundation) => "system lib".to_string(),
        Some(Category::App) => "you".to_string(),
        Some(Category::PulledIn(root)) => format!("← {root}"),
        Some(Category::Untraced) => "auto".to_string(),
        None => "auto".to_string(),
    }
}
