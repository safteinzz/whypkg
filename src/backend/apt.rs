//! The apt/dpkg backend — Debian, Ubuntu, Mint, and friends.
//!
//! It builds the entire [`World`] from a few bulk queries, the same data
//! sources the original `apt-why`/`apt-pending` scripts used, but parsed
//! in-process (no awk/grep/zcat subprocesses) and only once:
//!
//!   * `apt-mark showmanual`         → which packages the user chose
//!   * `dpkg-query -W` (metadata)    → version, size, synopsis
//!   * `dpkg-query -W` (deps)        → the dependency graph
//!   * `apt list --upgradable`       → pending upgrades
//!   * `/var/log/dpkg.log*`          → install dates + "same session"

use super::{Backend, capture};
use crate::model::{Package, World};
use chrono::NaiveDateTime;
use flate2::read::GzDecoder;
use std::collections::{HashMap, HashSet};
use std::io::Read;

pub struct Apt;

impl Backend for Apt {
    fn name(&self) -> &'static str {
        "apt"
    }

    fn build_world(&self) -> Result<World, String> {
        // ── manually-installed set ───────────────────────────────────────────
        let manual: HashSet<String> = capture("apt-mark", &["showmanual"])?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();

        // ── core metadata: name, version, size, synopsis ─────────────────────
        // `${Description}` carries the full multi-line description, so a single
        // record can span several output lines. Only the first line of a record
        // has the 4 tab-separated fields; continuation lines (the long
        // description) have no tabs, so we recognise records by field count.
        let meta_raw = capture(
            "dpkg-query",
            &[
                "-W",
                "-f=${Package}\t${Version}\t${Installed-Size}\t${Description}\n",
            ],
        )?;

        let mut packages: HashMap<String, Package> = HashMap::new();
        for line in meta_raw.lines() {
            let fields: Vec<&str> = line.splitn(4, '\t').collect();
            if fields.len() < 4 {
                continue; // long-description continuation line — ignore
            }
            let name = fields[0].to_string();
            if name.is_empty() {
                continue;
            }
            packages.insert(
                name.clone(),
                Package {
                    name,
                    version: fields[1].to_string(),
                    candidate: None,
                    installed_size: fields[2].trim().parse().unwrap_or(0),
                    description: fields[3].to_string(),
                    manual: false, // filled in below
                    install_epoch: None,
                    install_date: None,
                },
            );
        }

        // Mark manual packages now that the set and the map both exist.
        for (name, pkg) in packages.iter_mut() {
            pkg.manual = manual.contains(name);
        }

        // ── dependency graph (Depends + Recommends) ──────────────────────────
        let dep_raw = capture(
            "dpkg-query",
            &["-W", "-f=${Package}\t${Depends}\t${Recommends}\n"],
        )?;

        let mut deps: HashMap<String, Vec<String>> = HashMap::new();
        let mut rdeps: HashMap<String, Vec<String>> = HashMap::new();
        for line in dep_raw.lines() {
            let f: Vec<&str> = line.splitn(3, '\t').collect();
            if f.is_empty() || f[0].is_empty() {
                continue;
            }
            let pkg = f[0];
            let combined = format!(
                "{},{}",
                f.get(1).copied().unwrap_or(""),
                f.get(2).copied().unwrap_or("")
            );
            let mut seen = HashSet::new();
            for dep in parse_dep_field(&combined) {
                if dep == pkg || !seen.insert(dep.clone()) {
                    continue;
                }
                deps.entry(pkg.to_string()).or_default().push(dep.clone());
                rdeps.entry(dep).or_default().push(pkg.to_string());
            }
        }

        // ── pending upgrades ─────────────────────────────────────────────────
        // `apt list --upgradable` lines look like:
        //   pkg/suite 2.0 amd64 [upgradable from: 1.0]
        if let Ok(up_raw) = capture("apt", &["list", "--upgradable"]) {
            for line in up_raw.lines() {
                if line.starts_with("Listing") || line.is_empty() {
                    continue;
                }
                let name = match line.split('/').next() {
                    Some(n) if !n.is_empty() => n,
                    _ => continue,
                };
                let candidate = line.split_whitespace().nth(1).unwrap_or("");
                if let Some(pkg) = packages.get_mut(name)
                    && !candidate.is_empty()
                {
                    pkg.candidate = Some(candidate.to_string());
                }
            }
        }

        // ── install history (dates + "same session") ─────────────────────────
        let install_log = parse_dpkg_logs();
        // Attach each package's earliest install time for the dossier display.
        for (epoch, name) in &install_log {
            if let Some(pkg) = packages.get_mut(name)
                && pkg.install_epoch.is_none()
            {
                pkg.install_epoch = Some(*epoch);
                pkg.install_date = Some(epoch_to_date(*epoch));
            }
        }

        Ok(World {
            packages,
            deps,
            rdeps,
            manual,
            install_log,
        })
    }
}

/// Parse a combined `Depends,Recommends` field into bare package names.
///
/// Handles the three things dpkg packs in: version constraints `(>= 1.2)`,
/// alternatives `a | b` (we keep the first, like the original), and multiarch
/// qualifiers `name:amd64` (we strip the arch so it matches the plain names in
/// the package map).
fn parse_dep_field(field: &str) -> Vec<String> {
    field
        .split(',')
        .filter_map(|raw| {
            let mut s = raw.trim();
            if let Some(i) = s.find('|') {
                s = s[..i].trim(); // keep first alternative only
            }
            if let Some(i) = s.find('(') {
                s = s[..i].trim(); // drop version constraint
            }
            let name = s.split(':').next().unwrap_or(s).trim(); // drop :arch
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect()
}

/// Read every dpkg log (current, rotated, and gzipped) once and return all
/// `install` events as `(epoch, package)`, sorted oldest-first. This replaces
/// the bash version's habit of re-grepping ~1.6 MB of logs on every package
/// open — we do it a single time at startup.
fn parse_dpkg_logs() -> Vec<(i64, String)> {
    let mut events: Vec<(i64, String)> = Vec::new();

    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/var/log") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("dpkg.log") {
                paths.push(entry.path());
            }
        }
    }

    for path in paths {
        let content = match read_maybe_gz(&path) {
            Some(c) => c,
            None => continue,
        };
        for line in content.lines() {
            // "2026-06-14 02:57:01 install pkg:amd64 <none> 1.0"
            let mut it = line.split_whitespace();
            let (date, time, action) = match (it.next(), it.next(), it.next()) {
                (Some(d), Some(t), Some(a)) => (d, t, a),
                _ => continue,
            };
            if action != "install" {
                continue;
            }
            let pkg = match it.next() {
                Some(p) => p.split(':').next().unwrap_or(p).to_string(),
                None => continue,
            };
            if let Some(epoch) = parse_log_timestamp(date, time) {
                events.push((epoch, pkg));
            }
        }
    }

    events.sort_by_key(|(epoch, _)| *epoch);
    events
}

/// Read a file, transparently decompressing it if it's gzipped (`.gz`).
fn read_maybe_gz(path: &std::path::Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if path.extension().map(|e| e == "gz").unwrap_or(false) {
        let mut s = String::new();
        GzDecoder::new(&bytes[..]).read_to_string(&mut s).ok()?;
        Some(s)
    } else {
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Parse a dpkg log "YYYY-MM-DD HH:MM:SS" pair into a unix timestamp. We treat
/// it as UTC: only relative differences matter for the "same session" windows,
/// so the constant offset cancels out.
fn parse_log_timestamp(date: &str, time: &str) -> Option<i64> {
    NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc().timestamp())
}

/// Render a stored install epoch back to a `YYYY-MM-DD` string for display.
fn epoch_to_date(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
