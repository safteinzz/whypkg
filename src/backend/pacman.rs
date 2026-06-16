//! The pacman backend — Arch Linux and derivatives.
//!
//! pacman is in some ways friendlier than dpkg: a single `pacman -Qi` dump gives
//! us everything per package, including both `Depends On` *and* `Required By`
//! (so we don't have to invert the graph ourselves) and `Install Reason`
//! (explicit vs. dependency, i.e. manual vs. auto). Upgrades come from
//! `pacman -Qu` and install history from `/var/log/pacman.log`.
//!
//! The parsing lives in pure functions (`parse_qi`, `parse_log`,
//! `apply_upgradable`) so it can be unit-tested against captured real output
//! without needing pacman on the build machine — see the tests at the bottom.

use super::{capture, Backend};
use crate::model::{Package, World};
use chrono::NaiveDateTime;
use std::collections::HashMap;

pub struct Pacman;

impl Backend for Pacman {
    fn name(&self) -> &'static str {
        "pacman"
    }

    fn build_world(&self) -> Result<World, String> {
        // One bulk call: full info for every installed package.
        let qi = capture("pacman", &["-Qi"])?;
        let mut world = parse_qi(&qi);

        // Pending upgrades. Like `apt list --upgradable`, this reflects the last
        // database sync (`pacman -Sy`) — we never sync ourselves (needs root).
        if let Ok(qu) = capture("pacman", &["-Qu"]) {
            apply_upgradable(&mut world, &qu);
        }

        // Install history for the "same session" clue.
        if let Ok(log) = std::fs::read_to_string("/var/log/pacman.log") {
            world.install_log = parse_log(&log);
        }

        Ok(world)
    }
}

/// Parse a full `pacman -Qi` dump (run under `LC_ALL=C`) into a [`World`].
/// Packages are separated by blank lines; within a block each field is
/// `Key            : value`, with some fields wrapping onto indented lines we
/// don't need. Builds `deps`/`rdeps` directly from pacman's own
/// `Depends On`/`Required By`.
pub fn parse_qi(text: &str) -> World {
    let mut packages: HashMap<String, Package> = HashMap::new();
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    let mut rdeps: HashMap<String, Vec<String>> = HashMap::new();

    for block in text.split("\n\n") {
        if block.trim().is_empty() {
            continue;
        }

        let mut name = String::new();
        let mut version = String::new();
        let mut description = String::new();
        let mut size_kb = 0u64;
        let mut manual = false;
        let mut install_epoch = None;
        let mut install_date = None;
        let mut depends: Vec<String> = Vec::new();
        let mut required: Vec<String> = Vec::new();

        for line in block.lines() {
            let (key, value) = match field(line) {
                Some(kv) => kv,
                None => continue, // continuation line (e.g. Optional Deps)
            };
            match key {
                "Name" => name = value.to_string(),
                "Version" => version = value.to_string(),
                "Description" => description = value.to_string(),
                "Installed Size" => size_kb = parse_size_kb(value),
                "Depends On" => depends = parse_pkg_list(value),
                "Required By" => required = parse_pkg_list(value),
                "Install Reason" => manual = value.starts_with("Explicitly"),
                "Install Date" => {
                    if let Some((epoch, date)) = parse_pacman_date(value) {
                        install_epoch = Some(epoch);
                        install_date = Some(date);
                    }
                }
                _ => {}
            }
        }

        if name.is_empty() {
            continue;
        }
        if !depends.is_empty() {
            deps.insert(name.clone(), depends);
        }
        if !required.is_empty() {
            rdeps.insert(name.clone(), required);
        }
        packages.insert(
            name.clone(),
            Package {
                name,
                version,
                candidate: None,
                installed_size: size_kb,
                description,
                manual,
                install_epoch,
                install_date,
            },
        );
    }

    let manual = packages
        .values()
        .filter(|p| p.manual)
        .map(|p| p.name.clone())
        .collect();

    World {
        packages,
        deps,
        rdeps,
        manual,
        install_log: Vec::new(),
    }
}

/// Apply `pacman -Qu` output (`name oldver -> newver`) as pending upgrades.
pub fn apply_upgradable(world: &mut World, qu: &str) {
    for line in qu.lines() {
        let name = match line.split_whitespace().next() {
            Some(n) if !n.is_empty() => n,
            _ => continue,
        };
        let candidate = line.split("->").nth(1).map(str::trim).unwrap_or("");
        if let Some(pkg) = world.packages.get_mut(name) {
            if !candidate.is_empty() {
                pkg.candidate = Some(candidate.to_string());
            }
        }
    }
}

/// Parse `/var/log/pacman.log` install events into `(epoch, package)`, sorted.
/// Lines look like: `[2026-06-14T00:03:53+0000] [ALPM] installed glibc (2.43…)`.
pub fn parse_log(text: &str) -> Vec<(i64, String)> {
    let mut events: Vec<(i64, String)> = Vec::new();
    for line in text.lines() {
        let (ts, rest) = match line.split_once("] [ALPM] installed ") {
            Some(pair) => pair,
            None => continue,
        };
        let ts = ts.trim_start_matches('[');
        let name = match rest.split_whitespace().next() {
            Some(n) if !n.is_empty() => n,
            _ => continue,
        };
        if let Ok(dt) = chrono::DateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%z") {
            events.push((dt.timestamp(), name.to_string()));
        }
    }
    events.sort_by_key(|(epoch, _)| *epoch);
    events
}

/// Split a field line `Key            : value` into `(key, value)`. Returns
/// `None` for indented continuation lines, so they're skipped.
fn field(line: &str) -> Option<(&str, &str)> {
    if line.starts_with(' ') {
        return None;
    }
    let (k, v) = line.split_once(" : ")?;
    Some((k.trim(), v))
}

/// Parse a space-separated package list as printed by pacman, stripping version
/// constraints (`glibc>=2.34` → `glibc`) and treating "None" as empty.
fn parse_pkg_list(value: &str) -> Vec<String> {
    if value.trim() == "None" {
        return Vec::new();
    }
    value
        .split_whitespace()
        .map(|tok| {
            let end = tok
                .find(|c| c == '=' || c == '<' || c == '>')
                .unwrap_or(tok.len());
            tok[..end].to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse an "Installed Size" value like "5.37 MiB" into KB (to match the unit
/// the rest of the tool uses).
fn parse_size_kb(value: &str) -> u64 {
    let mut it = value.split_whitespace();
    let amount: f64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let mult = match it.next().unwrap_or("") {
        "B" => 1.0 / 1024.0,
        "KiB" | "KB" => 1.0,
        "MiB" | "MB" => 1024.0,
        "GiB" | "GB" => 1024.0 * 1024.0,
        _ => 1.0,
    };
    (amount * mult) as u64
}

/// Parse pacman's `LC_ALL=C` install date ("Tue Jun 16 17:33:00 2026") into a
/// `(unix_epoch, "YYYY-MM-DD")` pair.
fn parse_pacman_date(value: &str) -> Option<(i64, String)> {
    let dt = NaiveDateTime::parse_from_str(value.trim(), "%a %b %e %T %Y").ok()?;
    Some((dt.and_utc().timestamp(), dt.format("%Y-%m-%d").to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `pacman -Qi` / pacman.log output captured from an Arch container.
    const QI: &str = include_str!("../../tests/fixtures/pacman_qi.txt");
    const LOG: &str = include_str!("../../tests/fixtures/pacman_log.txt");

    #[test]
    fn parses_all_packages() {
        let world = parse_qi(QI);
        // The fixture was captured as `base + vim + git`.
        assert_eq!(world.packages.len(), 146);
        assert!(world.packages.contains_key("vim"));
        assert!(world.packages.contains_key("glibc"));
    }

    #[test]
    fn manual_vs_auto_from_install_reason() {
        let world = parse_qi(QI);
        assert!(world.is_manual("vim"), "vim was explicitly installed");
        assert!(!world.is_manual("glibc"), "glibc is a dependency");
    }

    #[test]
    fn graph_uses_native_depends_and_required_by() {
        let world = parse_qi(QI);
        // vim depends on glibc (version constraint stripped)…
        assert!(world.deps_of("vim").iter().any(|d| d == "glibc"));
        // …and glibc is, conversely, required by vim.
        assert!(world.rdeps_of("glibc").iter().any(|r| r == "vim"));
    }

    #[test]
    fn parses_size_and_date() {
        let world = parse_qi(QI);
        let vim = &world.packages["vim"];
        assert_eq!(vim.installed_size, 5498); // 5.37 MiB → KB
        assert!(vim.install_date.is_some());
        assert!(vim.install_epoch.unwrap() > 0);
    }

    #[test]
    fn parses_install_log() {
        let events = parse_log(LOG);
        assert!(!events.is_empty());
        assert!(events.iter().any(|(_, name)| name == "glibc"));
        // Sorted oldest-first.
        assert!(events.windows(2).all(|w| w[0].0 <= w[1].0));
    }

    #[test]
    fn upgradable_parsing() {
        let mut world = parse_qi(QI);
        apply_upgradable(&mut world, "vim 9.2.0653-1 -> 9.2.0700-1\n");
        assert_eq!(
            world.packages["vim"].candidate.as_deref(),
            Some("9.2.0700-1")
        );
    }

    #[test]
    fn size_units() {
        assert_eq!(parse_size_kb("512.00 KiB"), 512);
        assert_eq!(parse_size_kb("2.00 MiB"), 2048);
        assert_eq!(parse_size_kb("1.00 GiB"), 1024 * 1024);
    }
}
