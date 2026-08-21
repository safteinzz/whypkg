//! The in-memory model: one `Package` per installed package, plus the `World`
//! that holds every package and the full dependency graph.
//!
//! The whole point of the Rust rewrite is here: a backend builds this `World`
//! exactly once at startup (a handful of bulk queries), and from then on every
//! lookup during navigation - reverse-deps, "why is this here", install date -
//! is an in-memory `HashMap` hit. The old bash re-`grep`ped ~1.6 MB of logs on
//! *every* dossier open; we parse everything once and never touch a subprocess
//! again while the user is browsing.

use std::collections::{HashMap, HashSet};

/// Everything we know about a single installed package.
/// Which packaging system a package belongs to. The system package manager
/// (apt/dnf/pacman) and Flatpak coexist, so a `World` can hold both.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// The distro's package manager (apt, dnf, pacman).
    System,
    /// A Flatpak app.
    Flatpak,
}

/// Where an installed package came from, as best the backend can determine.
/// Answers "is this from my repos, or did I sideload it?".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    /// Offered by a currently-configured repository (the normal case).
    Repo,
    /// Installed from a local file (a `.deb`/`.rpm`, an AUR or local build).
    Local,
    /// Installed, but no configured repo offers it anymore (repo removed).
    Orphaned,
    /// Could not be determined.
    Unknown,
}

#[derive(Clone)]
pub struct Package {
    pub name: String,
    pub version: String,
    /// The version we'd upgrade to, if this package is upgradable.
    pub candidate: Option<String>,
    /// Installed size in KB, as the package manager reports it.
    pub installed_size: u64,
    /// Short synopsis (first line of the description only).
    pub description: String,
    /// Longer description text, when the manager exposes it (apt's extended
    /// description). Not shown in the list, but searchable and shown in the
    /// dossier - so a package like `code` is findable by "visual studio".
    pub details: Option<String>,
    /// Explicitly installed by the user (`true`) vs. pulled in as a dependency.
    pub manual: bool,
    /// Which packaging system this belongs to (system PM vs Flatpak).
    pub source: Source,
    /// For Flatpak, the remote it came from (e.g. "flathub"); `None` for system
    /// packages.
    pub remote: Option<String>,
    /// Where the package came from (repo / local file / orphaned).
    pub origin: Origin,
    /// Unix timestamp it was installed, parsed from the package-manager log.
    /// Used for relative "same session" window math (timezone cancels out).
    pub install_epoch: Option<i64>,
    /// Human-readable install date (`YYYY-MM-DD`) for display.
    pub install_date: Option<String>,
}

/// The complete picture of the system: packages + the dependency graph, built
/// once by a [`crate::backend::Backend`].
pub struct World {
    /// Every installed package, keyed by name.
    pub packages: HashMap<String, Package>,
    /// `deps[p]` = packages `p` depends on (Depends + Recommends).
    pub deps: HashMap<String, Vec<String>>,
    /// `rdeps[p]` = packages that depend on `p` (the reverse edges).
    pub rdeps: HashMap<String, Vec<String>>,
    /// Manually-installed package names, kept as a set for O(1) membership
    /// (the bash version did O(n) substring scans of a giant string here).
    pub manual: HashSet<String>,
    /// Every install event from the package-manager log, oldest first:
    /// `(epoch, package)`. Powers the "installed in the same session" clue.
    pub install_log: Vec<(i64, String)>,
}

#[cfg(test)]
impl World {
    /// Build a small synthetic world for tests, from `(dependent, dependency)`
    /// edges plus the set of manually-installed names. Both directions of the
    /// graph are filled in, so it behaves like a real backend's output.
    pub fn from_edges(edges: &[(&str, &str)], manual: &[&str]) -> World {
        let mut world = World::empty();
        let add = |world: &mut World, name: &str| {
            world
                .packages
                .entry(name.to_string())
                .or_insert_with(|| Package {
                    name: name.to_string(),
                    version: "1.0".into(),
                    candidate: None,
                    installed_size: 100,
                    description: format!("{name} description"),
                    details: None,
                    manual: false,
                    source: Source::System,
                    remote: None,
                    origin: Origin::Repo,
                    install_epoch: None,
                    install_date: None,
                });
        };
        for (dependent, dependency) in edges {
            add(&mut world, dependent);
            add(&mut world, dependency);
            world
                .deps
                .entry(dependent.to_string())
                .or_default()
                .push(dependency.to_string());
            world
                .rdeps
                .entry(dependency.to_string())
                .or_default()
                .push(dependent.to_string());
        }
        for name in manual {
            add(&mut world, name);
            world.manual.insert(name.to_string());
            if let Some(p) = world.packages.get_mut(*name) {
                p.manual = true;
            }
        }
        world
    }
}

impl World {
    /// An empty world - used when there's no system package manager but Flatpak
    /// is present, so we can still show flatpak apps.
    pub fn empty() -> Self {
        World {
            packages: HashMap::new(),
            deps: HashMap::new(),
            rdeps: HashMap::new(),
            manual: HashSet::new(),
            install_log: Vec::new(),
        }
    }

    pub fn is_manual(&self, pkg: &str) -> bool {
        self.manual.contains(pkg)
    }

    pub fn is_upgradable(&self, pkg: &str) -> bool {
        self.packages
            .get(pkg)
            .map(|p| p.candidate.is_some())
            .unwrap_or(false)
    }

    /// Packages that depend on `pkg` (what would break if it were removed).
    pub fn rdeps_of(&self, pkg: &str) -> &[String] {
        self.rdeps.get(pkg).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Packages `pkg` depends on.
    pub fn deps_of(&self, pkg: &str) -> &[String] {
        self.deps.get(pkg).map(Vec::as_slice).unwrap_or(&[])
    }

    /// How many packages depend on `pkg`. High counts mark foundational
    /// system libraries (libc, etc.) rather than things you'd ever remove.
    pub fn rdep_count(&self, pkg: &str) -> usize {
        self.rdeps.get(pkg).map(Vec::len).unwrap_or(0)
    }

    /// All installed package names, sorted - the default browse pool.
    pub fn all_names_sorted(&self) -> Vec<String> {
        let mut names: Vec<String> = self.packages.keys().cloned().collect();
        names.sort();
        names
    }

    /// Upgradable package names, sorted - the `--upgradable` browse pool.
    pub fn upgradable_names_sorted(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .packages
            .values()
            .filter(|p| p.candidate.is_some())
            .map(|p| p.name.clone())
            .collect();
        names.sort();
        names
    }
}
