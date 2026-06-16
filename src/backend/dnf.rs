//! The dnf/rpm backend — Fedora, RHEL, and derivatives.
//!
//! rpm is the awkward one: dependencies are expressed as *capabilities*
//! (`libc.so.6()(64bit)`, `config(bash)`, `/usr/bin/sh`), not package names. So
//! we build a provider map (capability/file → package) from `PROVIDES` and
//! `FILENAMES`, then resolve each package's `REQUIRES` through it to get a real
//! package-to-package graph. Everything is bulk `rpm -qa` queries (local, no
//! network); only the manual set and upgrade list use dnf.
//!
//! As with the other backends the parsing is pure functions (`build`,
//! `parse_meta`, …) unit-tested against captured Fedora output.

use super::{capture, Backend};
use crate::model::{Package, World};
use std::collections::{HashMap, HashSet};

pub struct Dnf;

impl Backend for Dnf {
    fn name(&self) -> &'static str {
        "dnf"
    }

    fn build_world(&self) -> Result<World, String> {
        // rpm queryformat: NAME outside the `[ ]` array iterator (printed once),
        // then the array tag inside it.
        let meta = capture(
            "rpm",
            &["-qa", "--qf", "%{NAME}\t%{VERSION}-%{RELEASE}\t%{SIZE}\t%{INSTALLTIME}\t%{SUMMARY}\n"],
        )?;
        let provides = capture("rpm", &["-qa", "--qf", "%{NAME}\t[%{PROVIDENAME} ]\n"])?;
        let requires = capture("rpm", &["-qa", "--qf", "%{NAME}\t[%{REQUIRENAME} ]\n"])?;
        let files = capture("rpm", &["-qa", "--qf", "%{NAME}\t[%{FILENAMES} ]\n"])?;
        // Manual set + upgrades come from dnf. `--cacheonly` keeps us offline;
        // upgrades simply come back empty if the user hasn't refreshed metadata
        // (same contract as `apt list --upgradable` needing `apt update`).
        let userinstalled =
            capture("dnf", &["repoquery", "--userinstalled", "--qf", "%{name}\n", "--cacheonly"])
                .unwrap_or_default();

        let mut world = build(&meta, &provides, &requires, &files, &userinstalled);

        if let Ok(up) = capture(
            "dnf",
            &["repoquery", "--upgrades", "--qf", "%{name}\t%{evr}\n", "--cacheonly"],
        ) {
            apply_upgradable(&mut world, &up);
        }

        Ok(world)
    }
}

/// Assemble a [`World`] from the raw rpm/dnf dumps. Pure so it can be tested.
pub fn build(
    meta: &str,
    provides: &str,
    requires: &str,
    files: &str,
    userinstalled: &str,
) -> World {
    let manual_set: HashSet<String> = userinstalled
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();

    let (mut packages, install_log) = parse_meta(meta);
    for (name, pkg) in packages.iter_mut() {
        pkg.manual = manual_set.contains(name);
    }

    let provider = provider_map(provides, files);
    let (deps, rdeps) = resolve_graph(requires, &provider);

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
        install_log,
    }
}

/// Parse the metadata dump into packages and the install timeline. Each line is
/// `name<TAB>version<TAB>size_bytes<TAB>installtime_epoch<TAB>summary`.
pub fn parse_meta(meta: &str) -> (HashMap<String, Package>, Vec<(i64, String)>) {
    let mut packages = HashMap::new();
    let mut install_log: Vec<(i64, String)> = Vec::new();

    for line in meta.lines() {
        let f: Vec<&str> = line.splitn(5, '\t').collect();
        if f.len() < 5 || f[0].is_empty() {
            continue;
        }
        let name = f[0].to_string();
        let size_kb = f[2].trim().parse::<u64>().unwrap_or(0) / 1024;
        let epoch = f[3].trim().parse::<i64>().ok();
        let (install_epoch, install_date) = match epoch {
            Some(e) if e > 0 => {
                let date = chrono::DateTime::from_timestamp(e, 0)
                    .map(|dt| dt.format("%Y-%m-%d").to_string());
                install_log.push((e, name.clone()));
                (Some(e), date)
            }
            _ => (None, None),
        };
        packages.insert(
            name.clone(),
            Package {
                name,
                version: f[1].to_string(),
                candidate: None,
                installed_size: size_kb,
                description: f[4].to_string(),
                manual: false,
                install_epoch,
                install_date,
            },
        );
    }

    install_log.sort_by_key(|(e, _)| *e);
    (packages, install_log)
}

/// Build `capability/file → providing package` from the PROVIDES and FILENAMES
/// dumps. First provider wins on ties (good enough for origin tracing).
fn provider_map(provides: &str, files: &str) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    for (pkg, caps) in name_caps(provides).into_iter().chain(name_caps(files)) {
        for cap in caps {
            map.entry(cap).or_insert_with(|| pkg.clone());
        }
    }
    map
}

/// Resolve every package's REQUIRES through the provider map into a real
/// package-to-package graph (`deps` and its inverse `rdeps`).
fn resolve_graph(
    requires: &str,
    provider: &HashMap<String, String>,
) -> (HashMap<String, Vec<String>>, HashMap<String, Vec<String>>) {
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    let mut rdeps: HashMap<String, Vec<String>> = HashMap::new();

    for (pkg, reqs) in name_caps(requires) {
        let mut seen = HashSet::new();
        for req in reqs {
            // rpmlib()/rtld() are internal rpm features, not real packages.
            if req.starts_with("rpmlib(") || req.starts_with("rtld(") {
                continue;
            }
            if let Some(prov) = provider.get(&req) {
                if prov != &pkg && seen.insert(prov.clone()) {
                    deps.entry(pkg.clone()).or_default().push(prov.clone());
                    rdeps.entry(prov.clone()).or_default().push(pkg.clone());
                }
            }
        }
    }
    (deps, rdeps)
}

/// Apply `dnf repoquery --upgrades` output (`name<TAB>evr`) as pending upgrades.
pub fn apply_upgradable(world: &mut World, up: &str) {
    for line in up.lines() {
        let (name, evr) = match line.split_once('\t') {
            Some(pair) => pair,
            None => continue,
        };
        if let Some(pkg) = world.packages.get_mut(name.trim()) {
            let evr = evr.trim();
            if !evr.is_empty() {
                pkg.candidate = Some(evr.to_string());
            }
        }
    }
}

/// Parse a `NAME<TAB>cap1 cap2 …` dump into `(package, capabilities)` pairs.
fn name_caps(text: &str) -> Vec<(String, Vec<String>)> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once('\t')?;
            if name.is_empty() {
                return None;
            }
            let caps = rest.split_whitespace().map(String::from).collect();
            Some((name.to_string(), caps))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: &str = include_str!("../../tests/fixtures/dnf_meta.txt");
    const PROVIDES: &str = include_str!("../../tests/fixtures/dnf_provides.txt");
    const REQUIRES: &str = include_str!("../../tests/fixtures/dnf_requires.txt");
    const FILES: &str = include_str!("../../tests/fixtures/dnf_files.txt");
    const USERINSTALLED: &str = include_str!("../../tests/fixtures/dnf_userinstalled.txt");

    fn world() -> World {
        build(META, PROVIDES, REQUIRES, FILES, USERINSTALLED)
    }

    #[test]
    fn parses_all_packages() {
        let w = world();
        assert_eq!(w.packages.len(), 241);
        assert!(w.packages.contains_key("bash"));
        assert!(w.packages.contains_key("glibc"));
    }

    #[test]
    fn meta_size_and_installtime() {
        let w = world();
        let bash = &w.packages["bash"];
        assert!(bash.installed_size > 0); // SIZE bytes → KB
        assert!(bash.install_epoch.unwrap() > 0); // rpm INSTALLTIME
        assert!(bash.install_date.is_some());
    }

    #[test]
    fn resolves_capability_deps_to_packages() {
        let w = world();
        // bash REQUIRES libc.so.6(...) which glibc PROVIDES → edge bash→glibc.
        assert!(
            w.deps_of("bash").iter().any(|d| d == "glibc"),
            "bash should depend on glibc via resolved soname"
        );
        assert!(
            w.rdeps_of("glibc").iter().any(|r| r == "bash"),
            "glibc should be required by bash"
        );
    }

    #[test]
    fn manual_set_from_userinstalled() {
        let w = world();
        // vim was explicitly installed in the fixture capture.
        assert!(w.is_manual("vim-minimal") || w.is_manual("vim"));
        // a pulled-in library should not be manual.
        assert!(!w.is_manual("glibc"));
    }

    #[test]
    fn install_log_sorted_and_populated() {
        let w = world();
        assert!(!w.install_log.is_empty());
        assert!(w.install_log.windows(2).all(|x| x[0].0 <= x[1].0));
    }

    #[test]
    fn upgradable_parsing() {
        let mut w = world();
        apply_upgradable(&mut w, "bash\t5.3.0-2.fc44\n");
        assert_eq!(w.packages["bash"].candidate.as_deref(), Some("5.3.0-2.fc44"));
    }
}
