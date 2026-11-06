//! `npm install -g`: each global package and the packages in its own
//! `node_modules`.
//!
//! Read from the `package.json` files on disk rather than `npm ls -g`, which
//! costs a node start. The node every global package runs on is a `uses` edge
//! to the system package that owns it.

use super::{Backend, Layer, home, mtime, which};
use crate::model::{Origin, Package, Source};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The fields of a `package.json` that whypkg reads.
#[derive(Debug, Default, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    /// `dependencies`, `optionalDependencies` and `peerDependencies`: npm
    /// installs all three.
    pub requires: Vec<String>,
}

/// Every global package. `backend` places the global folders that sit inside
/// the system's own `/usr`, where the package manager may have put a node
/// module of its own (Arch's `typescript`), which is then a system package
/// and is left out here.
pub fn read(backend: Option<&dyn Backend>) -> Layer {
    let node = which("node");
    // npm and corepack ship in node's own global folder; nobody installed
    // them with `npm -g`.
    let own_root = node
        .as_deref()
        .and_then(|n| n.parent()?.parent())
        .and_then(|prefix| prefix.join("lib/node_modules").canonicalize().ok());
    let mut layer = Layer::default();
    let mut taken: HashSet<String> = HashSet::new();
    for root in global_roots(node.as_deref()) {
        let bundled = own_root.as_ref() == Some(&root);
        let mut tops: Vec<(String, PathBuf)> = Vec::new();
        for dir in package_dirs(&root) {
            let Some(manifest) = read_manifest(&dir) else {
                continue;
            };
            if bundled && matches!(manifest.name.as_str(), "npm" | "corepack") {
                continue;
            }
            // A package in two global folders (an old prefix left behind)
            // keeps both copies apart by version.
            let mut key = format!("npm:{}", manifest.name);
            if taken.contains(&key) {
                key = format!("{key}@{}", manifest.version);
            }
            if !taken.insert(key.clone()) {
                continue;
            }
            tops.push((key, dir));
        }
        if root.starts_with("/usr") && !root.starts_with("/usr/local") {
            let manifests: Vec<PathBuf> = tops
                .iter()
                .filter_map(|(_, dir)| dir.join("package.json").canonicalize().ok())
                .collect();
            let owned = backend.map(|b| b.owners(&manifests)).unwrap_or_default();
            tops.retain(|(_, dir)| {
                dir.join("package.json")
                    .canonicalize()
                    .map_or(true, |m| !owned.contains_key(&m))
            });
        }
        for (key, dir) in tops {
            read_tree(&key, &dir, node.as_deref(), &mut layer);
        }
    }
    layer
}

/// The global `node_modules` folders: npm's configured prefix, then node's
/// own prefix, then `/usr/local`, where Debian's and Fedora's npm install.
fn global_roots(node: Option<&Path>) -> Vec<PathBuf> {
    let mut prefixes: Vec<PathBuf> = Vec::new();
    for var in ["NPM_CONFIG_PREFIX", "npm_config_prefix"] {
        if let Some(p) = std::env::var_os(var).filter(|p| !p.is_empty()) {
            prefixes.push(PathBuf::from(p));
        }
    }
    if let Some(home) = home()
        && let Ok(rc) = std::fs::read_to_string(home.join(".npmrc"))
        && let Some(prefix) = npmrc_prefix(&rc, &home)
    {
        prefixes.push(prefix);
    }
    if let Some(prefix) = node.and_then(|n| n.parent()?.parent()) {
        prefixes.push(prefix.to_path_buf());
    }
    prefixes.push(PathBuf::from("/usr/local"));

    let mut roots: Vec<PathBuf> = Vec::new();
    for prefix in prefixes {
        if let Ok(root) = prefix.join("lib/node_modules").canonicalize()
            && !roots.contains(&root)
        {
            roots.push(root);
        }
    }
    roots
}

/// The `prefix=` line of an `.npmrc`, with `~` and `${HOME}` expanded.
pub fn npmrc_prefix(text: &str, home: &Path) -> Option<PathBuf> {
    let value = text.lines().find_map(|l| {
        let (key, value) = l.split_once('=')?;
        (key.trim() == "prefix").then(|| value.trim())
    })?;
    let home = home.to_string_lossy();
    let value = value.replace("${HOME}", &home);
    let value = match value.strip_prefix('~') {
        Some(rest) => format!("{home}{rest}"),
        None => value,
    };
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// Every package folder in one `node_modules`, scoped ones (`@types/node`)
/// included, skipping npm's own entries (`.bin`, `.package-lock.json`).
fn package_dirs(node_modules: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(node_modules) else {
        return out;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if name.starts_with('@') {
            out.extend(
                std::fs::read_dir(&path)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .map(|e| e.path()),
            );
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn read_manifest(dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let manifest = parse_manifest(&text)?;
    (!manifest.name.is_empty()).then_some(manifest)
}

/// Parse the fields whypkg reads out of a `package.json`.
pub fn parse_manifest(text: &str) -> Option<Manifest> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let field = |f: &str| {
        json.get(f)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let mut requires = Vec::new();
    for table in ["dependencies", "optionalDependencies", "peerDependencies"] {
        if let Some(deps) = json.get(table).and_then(|d| d.as_object()) {
            for name in deps.keys() {
                if !requires.contains(name) {
                    requires.push(name.clone());
                }
            }
        }
    }
    Some(Manifest {
        name: field("name"),
        version: field("version"),
        description: field("description"),
        requires,
    })
}

/// Read the global package keyed `top_key` in `dir` and everything under its
/// `node_modules`, each keyed `<top_key>/<name>`.
fn read_tree(top_key: &str, dir: &Path, node: Option<&Path>, layer: &mut Layer) {
    let top_key = top_key.to_string();
    // Folder → key, for resolving a dependency the way node does: the nearest
    // `node_modules` up the tree that holds it.
    let mut keys: HashMap<PathBuf, String> = HashMap::new();
    let mut found: Vec<(PathBuf, Manifest)> = Vec::new();
    let mut queue = vec![dir.to_path_buf()];
    // A symlinked package that points back up the tree would otherwise be
    // walked forever.
    let mut seen: HashSet<PathBuf> = HashSet::new();
    while let Some(pkg_dir) = queue.pop() {
        if !pkg_dir.canonicalize().is_ok_and(|real| seen.insert(real)) {
            continue;
        }
        let Some(manifest) = read_manifest(&pkg_dir) else {
            continue;
        };
        let key = if pkg_dir == dir {
            top_key.clone()
        } else {
            let plain = format!("{top_key}/{}", manifest.name);
            if keys.values().any(|k| *k == plain) {
                format!("{plain}@{}", manifest.version)
            } else {
                plain
            }
        };
        keys.insert(pkg_dir.clone(), key);
        queue.extend(package_dirs(&pkg_dir.join("node_modules")));
        found.push((pkg_dir, manifest));
    }

    let linked = std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink());
    let date = mtime(dir);
    for (pkg_dir, manifest) in &found {
        let key = &keys[pkg_dir];
        let is_top = pkg_dir == dir;
        for dep in &manifest.requires {
            if let Some(target) = resolve(pkg_dir, dir, dep).and_then(|d| keys.get(&d)) {
                layer.deps.push((key.clone(), target.clone()));
            }
        }
        layer.packages.push(Package {
            name: key.clone(),
            label: Some(manifest.name.clone()),
            version: manifest.version.clone(),
            candidate: None,
            // Rounded up, so a package of a few hundred bytes is not `n/a`.
            installed_size: own_size(pkg_dir).div_ceil(1024),
            description: manifest.description.clone(),
            details: None,
            manual: is_top,
            source: Source::Npm,
            remote: Some(if is_top && linked {
                dir.canonicalize()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                "the npm registry".to_string()
            }),
            origin: if is_top && linked {
                Origin::Local
            } else {
                Origin::Repo
            },
            install_epoch: date.as_ref().map(|(epoch, _)| *epoch),
            install_date: date.as_ref().map(|(_, day)| day.clone()),
        });
    }
    if let Some(node) = node {
        layer.needs.push((top_key, node.to_path_buf()));
    }
}

/// Where node finds `dep` for the package in `from`: its own `node_modules`,
/// then each enclosing package's, up to and including the global package
/// `top`.
fn resolve(from: &Path, top: &Path, dep: &str) -> Option<PathBuf> {
    let mut dir = from.to_path_buf();
    loop {
        let candidate = dir.join("node_modules").join(dep);
        if candidate.join("package.json").is_file() {
            return Some(candidate);
        }
        if dir == top {
            return None;
        }
        // Up out of `…/node_modules/<pkg>` (or `…/node_modules/@scope/<pkg>`)
        // to the package that holds that `node_modules`.
        let mut up = dir.parent()?;
        while up.file_name().is_some_and(|n| n != "node_modules") {
            up = up.parent()?;
        }
        dir = up.parent()?.to_path_buf();
    }
}

/// Bytes in a package's own files, leaving out its `node_modules`, whose
/// packages count their own.
fn own_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if entry.file_name() != "node_modules" {
                    stack.push(entry.path());
                }
            } else if kind.is_file() {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}
