//! The zypper backend: openSUSE and SLE.
//!
//! The packages and the graph are rpm's, built exactly as the dnf backend
//! builds them. zypper adds the rest from cached, offline queries: `search
//! --installed-only` marks what you installed (`i+`), `packages --orphaned`
//! what no repository offers, and `list-updates` the pending upgrades. All are
//! `|`-separated tables, parsed by pure functions.

use super::dnf::RpmDump;
use super::{Backend, capture_checked};
use crate::model::{Origin, World};

pub struct Zypper;

impl Backend for Zypper {
    fn name(&self) -> &'static str {
        "zypper"
    }

    fn unneeded(&self) -> Result<Vec<String>, String> {
        let out = zypper(&["packages", "--unneeded"])?;
        Ok(names(&out))
    }

    fn build_world(&self) -> Result<World, String> {
        let rpm = RpmDump::query()?;
        let manual = parse_manual(&zypper(&[
            "search",
            "--installed-only",
            "--type",
            "package",
        ])?);
        let mut world = rpm.build(&manual.join("\n"));

        // `(System Packages)` in a search only means no repository holds that
        // exact version, so the orphans come from zypper's own `--orphaned`.
        for pkg in world.packages.values_mut() {
            pkg.origin = Origin::Repo;
        }
        for name in names(&zypper(&["packages", "--orphaned"])?) {
            if let Some(pkg) = world.packages.get_mut(&name) {
                pkg.origin = Origin::Orphaned;
            }
        }

        if let Ok(up) = zypper(&["list-updates", "--type", "package"]) {
            for (name, available) in parse_updates(&up) {
                if let Some(pkg) = world.packages.get_mut(&name) {
                    pkg.candidate = Some(available);
                }
            }
        }

        Ok(world)
    }
}

/// Run a read-only zypper query on the cached metadata (`--no-refresh`), so
/// upgrades are as fresh as the user's last `zypper refresh`. zypper's exit
/// codes 100 to 104, 106 and 107 report something about the system (updates
/// waiting, nothing found, a repository without a cache skipped) rather than a
/// failed query.
fn zypper(args: &[&str]) -> Result<String, String> {
    let mut full = vec!["--non-interactive", "--no-refresh"];
    full.extend_from_slice(args);
    capture_checked("zypper", &full, &[100, 101, 102, 103, 104, 106, 107])
}

/// The cells of a zypper table row, or `None` for anything that is not one: a
/// banner line, the header, the `---+---` rule.
fn cells(line: &str) -> Option<Vec<&str>> {
    if !line.contains('|') {
        return None;
    }
    let cells: Vec<&str> = line.split('|').map(str::trim).collect();
    (cells.first() != Some(&"S")).then_some(cells)
}

/// The names in a `packages` table (`S | Repository | Name | Version | Arch`).
fn names(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(cells)
        .filter(|c| c.len() >= 5 && !c[2].is_empty())
        .map(|c| c[2].to_string())
        .collect()
}

/// The packages `search --installed-only` marks `i+`, installed because the
/// user asked for them (`S | Name | Summary | Type`).
pub fn parse_manual(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(cells)
        .filter(|c| c.len() >= 2 && c[0] == "i+" && !c[1].is_empty())
        .map(|c| c[1].to_string())
        .collect()
}

/// Parse `list-updates` (`S | Repository | Name | Current Version | Available
/// Version | Arch`) into `(name, available version)`.
pub fn parse_updates(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(cells)
        .filter(|c| c.len() >= 6 && !c[2].is_empty() && !c[4].is_empty())
        .map(|c| (c[2].to_string(), c[4].to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH: &str = include_str!("../../tests/fixtures/zypper_search_installed.txt");
    const ORPHANED: &str = include_str!("../../tests/fixtures/zypper_orphaned.txt");
    const UNNEEDED: &str = include_str!("../../tests/fixtures/zypper_unneeded.txt");
    const UPDATES: &str = include_str!("../../tests/fixtures/zypper_list_updates.txt");

    #[test]
    fn only_packages_marked_i_plus_count_as_installed_by_you() {
        let manual = parse_manual(SEARCH);
        assert_eq!(manual.len(), 28, "one name per `i+` row: {manual:?}");
        assert!(manual.contains(&"zypper".to_string()));
        assert!(
            !manual.contains(&"libbz2-1".to_string()),
            "an `i` row was taken as installed by you"
        );
        assert!(
            !manual.iter().any(|n| n == "Name" || n.starts_with('-')),
            "the header or the rule was read as a package: {manual:?}"
        );
    }

    #[test]
    fn a_packages_table_yields_exactly_its_name_column() {
        assert_eq!(names(UNNEEDED), vec!["libjq1".to_string()]);
        let orphaned = names(ORPHANED);
        assert_eq!(orphaned.len(), 119, "one name per row: {orphaned:?}");
        assert!(orphaned.contains(&"bash".to_string()));
        assert!(
            !orphaned.iter().any(|n| n == "Name" || n.starts_with('-')),
            "the header or the rule was read as a package: {orphaned:?}"
        );
    }

    #[test]
    fn list_updates_pairs_each_name_with_its_available_version() {
        assert_eq!(
            parse_updates(UPDATES),
            vec![("openSUSE-build-key".to_string(), "1.0-69.1".to_string())]
        );
    }
}
