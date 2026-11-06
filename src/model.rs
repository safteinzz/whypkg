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

/// Which installer put a package there. The distro's package manager and the
/// others coexist, so one `World` holds them all.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    /// The distro's package manager (apt, pacman, dnf, zypper).
    System,
    /// A Flatpak app or runtime.
    Flatpak,
    /// `cargo install`.
    Cargo,
    /// `pipx install`: a tool and the packages in its own virtualenv.
    Pipx,
    /// `uv tool install`: a tool and the packages in its own virtualenv.
    Uv,
    /// `npm install -g`: a package and the ones in its own `node_modules`.
    Npm,
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

impl Source {
    /// Every source, in the order the browser's tabs show them.
    pub const ALL: [Source; 6] = [
        Source::System,
        Source::Flatpak,
        Source::Cargo,
        Source::Pipx,
        Source::Uv,
        Source::Npm,
    ];

    /// The word `--json`, `--only` and the config file use for it.
    pub fn word(self) -> &'static str {
        match self {
            Source::System => "system",
            Source::Flatpak => "flatpak",
            Source::Cargo => "cargo",
            Source::Pipx => "pipx",
            Source::Uv => "uv",
            Source::Npm => "npm",
        }
    }

    pub fn from_word(word: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|s| s.word() == word)
    }

    /// The words of every source, for a message that lists them.
    pub fn words() -> String {
        Source::ALL.map(Source::word).join(", ")
    }
}

impl Origin {
    /// The word `--json` prints for it.
    pub fn word(self) -> &'static str {
        match self {
            Origin::Repo => "repo",
            Origin::Local => "local",
            Origin::Orphaned => "orphaned",
            Origin::Unknown => "unknown",
        }
    }
}

/// Everything we know about a single installed package.
#[derive(Clone)]
pub struct Package {
    /// The key it is stored under, unique across sources: the package
    /// manager's name for a system package, a flatpak ID, and `<source>:<name>`
    /// for the rest (`cargo:ripgrep`, `uv:ruff/click` for a package inside the
    /// `ruff` tool's virtualenv).
    pub name: String,
    /// The name a person knows it by when that is not `name` (`ripgrep` for
    /// `cargo:ripgrep`); read it through [`Package::label`].
    pub label: Option<String>,
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

impl Package {
    /// The name to show: `ripgrep` for `cargo:ripgrep`, the key itself for a
    /// system package.
    pub fn label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }
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
    /// `uses[p]` = system packages `p` loads or runs on: a library a cargo
    /// binary links, the Python or node a tool runs with. Kept out of `deps`
    /// because the package manager did not install them for `p`, so they must
    /// never answer "why is this here" or count toward a system library's
    /// dependents; they only warn that removing one breaks `p`.
    pub uses: HashMap<String, Vec<String>>,
    /// The reverse of `uses`.
    pub used_by: HashMap<String, Vec<String>>,
    /// The system package manager that built this world (`apt`), or `None`
    /// when there is none.
    pub manager: Option<&'static str>,
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
                    label: None,
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
            uses: HashMap::new(),
            used_by: HashMap::new(),
            manager: None,
        }
    }

    /// The name to show for the package stored under `key`.
    pub fn label<'a>(&'a self, key: &'a str) -> &'a str {
        self.packages.get(key).map_or(key, Package::label)
    }

    /// Which installer put the package stored under `key` there.
    pub fn source(&self, key: &str) -> Source {
        self.packages.get(key).map_or(Source::System, |p| p.source)
    }

    /// System packages `pkg` loads or runs on.
    pub fn uses_of(&self, pkg: &str) -> &[String] {
        self.uses.get(pkg).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Packages from other installers that load or run on `pkg`.
    pub fn used_by_of(&self, pkg: &str) -> &[String] {
        self.used_by.get(pkg).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Record that `user` loads or runs on the system package `pkg`.
    pub fn add_use(&mut self, user: &str, pkg: &str) {
        let uses = self.uses.entry(user.to_string()).or_default();
        if uses.iter().any(|u| u == pkg) {
            return;
        }
        uses.push(pkg.to_string());
        self.used_by
            .entry(pkg.to_string())
            .or_default()
            .push(user.to_string());
    }

    /// Record that `dependent` needs `dependency`, both already in `packages`.
    pub fn add_dep(&mut self, dependent: &str, dependency: &str) {
        let deps = self.deps.entry(dependent.to_string()).or_default();
        if deps.iter().any(|d| d == dependency) {
            return;
        }
        deps.push(dependency.to_string());
        self.rdeps
            .entry(dependency.to_string())
            .or_default()
            .push(dependent.to_string());
    }

    /// What needs `pkg`: the packages its installer pulled it in for, then the
    /// ones that load or run on it.
    pub fn needed_by_all(&self, pkg: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in self.rdeps_of(pkg).iter().chain(self.used_by_of(pkg)) {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        out
    }

    /// What `pkg` needs: its own dependencies, then the system packages it
    /// loads or runs on.
    pub fn depends_on_all(&self, pkg: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in self.deps_of(pkg).iter().chain(self.uses_of(pkg)) {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        out
    }

    /// The command that would remove `pkg`, for the dossier to show and never
    /// run, or `None` when its installer has no command for that one package
    /// (a package inside a tool's virtualenv goes when the tool does).
    pub fn remove_command(&self, pkg: &str) -> Option<String> {
        let p = self.packages.get(pkg)?;
        let tool = |prefix: &str| {
            let rest = pkg.strip_prefix(prefix)?;
            (!rest.contains('/')).then_some(rest)
        };
        Some(match p.source {
            Source::System => {
                let verb = match self.manager? {
                    "apt" => "apt remove",
                    "pacman" => "pacman -R",
                    "dnf" => "dnf remove",
                    "zypper" => "zypper remove",
                    _ => return None,
                };
                format!("sudo {verb} {pkg}")
            }
            Source::Flatpak if p.manual => format!("flatpak uninstall {pkg}"),
            Source::Flatpak => return None,
            Source::Cargo => format!("cargo uninstall {}", p.label()),
            Source::Pipx => format!("pipx uninstall {}", tool("pipx:")?),
            Source::Uv => format!("uv tool uninstall {}", tool("uv:")?),
            Source::Npm if p.manual => format!("npm uninstall -g {}", p.label()),
            Source::Npm => return None,
        })
    }

    /// Add a package, marking it manual when it is.
    pub fn insert(&mut self, pkg: Package) {
        if pkg.manual {
            self.manual.insert(pkg.name.clone());
        }
        self.packages.insert(pkg.name.clone(), pkg);
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
