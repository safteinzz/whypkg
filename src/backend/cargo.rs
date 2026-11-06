//! `cargo install`: the crates in `~/.cargo/.crates2.json`.
//!
//! A crate's own dependencies are compiled into its binaries, so nothing else
//! is installed for it. What it does need is the system libraries those
//! binaries load, and the programs its manifest says it runs
//! (`[package.metadata] runs`), which become `uses` edges to the system
//! packages that own them.

use super::elf::{self, Libraries};
use super::{Layer, home, mtime, which};
use crate::model::{Origin, Package, Source};
use std::path::{Path, PathBuf};

/// One crate `cargo install` records.
#[derive(Debug, PartialEq)]
pub struct Install {
    pub name: String,
    pub version: String,
    /// Where it was built from: `crates.io`, a git URL or a local path.
    pub from: From,
    pub bins: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum From {
    /// A registry, by the name a person knows it by (`crates.io`).
    Registry(String),
    Git(String),
    /// `cargo install --path`.
    Path(String),
}

/// `$CARGO_HOME`, or `~/.cargo`.
fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".cargo")))
}

/// Read every installed crate. A machine with no cargo installs, or a file
/// cargo has since changed the shape of, yields an empty layer.
pub fn read(libraries: &Libraries) -> Layer {
    let Some(root) = cargo_home() else {
        return Layer::default();
    };
    let Ok(text) = std::fs::read_to_string(root.join(".crates2.json")) else {
        return Layer::default();
    };
    let mut layer = Layer::default();
    for install in parse_crates2(&text) {
        let key = format!("cargo:{}", install.name);
        let bins: Vec<PathBuf> = install
            .bins
            .iter()
            .map(|b| root.join("bin").join(b))
            .filter(|p| p.is_file())
            .collect();
        let size: u64 = bins
            .iter()
            .filter_map(|b| std::fs::metadata(b).ok())
            .map(|m| m.len() / 1024)
            .sum();
        let date = bins.iter().find_map(|b| mtime(b));
        for bin in &bins {
            for soname in elf::needed(bin) {
                if let Some(path) = libraries.find(&soname) {
                    layer.needs.push((key.clone(), path));
                }
            }
        }
        let manifest = manifest(&root, &install)
            .map(|text| parse_manifest(&text))
            .unwrap_or_default();
        for program in &manifest.runs {
            if let Some(path) = which(program) {
                layer.needs.push((key.clone(), path));
            }
        }
        let (remote, origin) = match &install.from {
            From::Registry(name) => (name.clone(), Origin::Repo),
            From::Git(url) => (url.clone(), Origin::Repo),
            From::Path(path) => (path.clone(), Origin::Local),
        };
        layer.packages.push(Package {
            name: key,
            label: Some(install.name.clone()),
            description: manifest.description.unwrap_or_default(),
            details: Some(format!("binaries: {}", install.bins.join(", "))),
            version: install.version,
            candidate: None,
            installed_size: size,
            manual: true,
            source: Source::Cargo,
            remote: Some(remote),
            origin,
            install_epoch: date.as_ref().map(|(epoch, _)| *epoch),
            install_date: date.map(|(_, day)| day),
        });
    }
    layer
}

/// Parse `.crates2.json`, whose `installs` keys read
/// `ripgrep 14.1.0 (registry+https://github.com/rust-lang/crates.io-index)`.
pub fn parse_crates2(text: &str) -> Vec<Install> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(installs) = json.get("installs").and_then(|i| i.as_object()) else {
        return Vec::new();
    };
    let mut out: Vec<Install> = installs
        .iter()
        .filter_map(|(key, entry)| {
            let (name, rest) = key.split_once(' ')?;
            let (version, source) = rest.split_once(' ')?;
            let source = source.strip_prefix('(')?.strip_suffix(')')?;
            let bins = entry
                .get("bins")
                .and_then(|b| b.as_array())
                .map(|b| {
                    b.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            Some(Install {
                name: name.to_string(),
                version: version.to_string(),
                from: parse_source(source),
                bins,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `registry+https://…crates.io-index`, `sparse+https://index.crates.io/`,
/// `git+https://…#rev` or `path+file:///…`.
fn parse_source(source: &str) -> From {
    let (kind, url) = source.split_once('+').unwrap_or(("registry", source));
    match kind {
        "git" => From::Git(url.split('#').next().unwrap_or(url).to_string()),
        "path" => From::Path(url.strip_prefix("file://").unwrap_or(url).to_string()),
        _ if url.contains("crates.io") => From::Registry("crates.io".to_string()),
        _ => From::Registry(url.to_string()),
    }
}

/// The crate's `Cargo.toml`: from the folder it was built from for a `--path`
/// install, else the copy of its source cargo keeps after downloading it, when
/// that is still there. A `--git` install keeps none.
fn manifest(root: &Path, install: &Install) -> Option<String> {
    match &install.from {
        From::Registry(_) => {}
        From::Path(path) => {
            return std::fs::read_to_string(Path::new(path).join("Cargo.toml")).ok();
        }
        From::Git(_) => return None,
    }
    let dir = format!("{}-{}", install.name, install.version);
    let registries = std::fs::read_dir(root.join("registry/src")).ok()?;
    registries
        .filter_map(Result::ok)
        .map(|r| r.path().join(&dir).join("Cargo.toml"))
        .find_map(|manifest| std::fs::read_to_string(manifest).ok())
}

/// What whypkg reads from a crate's `Cargo.toml`.
#[derive(Debug, Default, PartialEq)]
pub struct Manifest {
    pub description: Option<String>,
    /// `[package.metadata] runs`: the programs the crate starts, by the name
    /// it runs them by (`ssh`), found on `PATH`.
    pub runs: Vec<String>,
}

pub fn parse_manifest(text: &str) -> Manifest {
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return Manifest::default();
    };
    let package = doc.get("package");
    let description = package
        .and_then(|p| p.get("description"))
        .and_then(|d| d.as_str())
        .map(|d| d.split_whitespace().collect::<Vec<_>>().join(" "));
    let runs = package
        .and_then(|p| p.get("metadata"))
        .and_then(|m| m.get("runs"))
        .and_then(|r| r.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Manifest { description, runs }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crates2_entry_yields_name_version_source_and_binaries() {
        let text = r#"{"installs":{
            "zeta-tool 2.1.0 (registry+https://github.com/rust-lang/crates.io-index)":{"bins":["zt"]},
            "alpha 0.3.0 (git+https://example.com/alpha.git#0123abcd)":{"bins":["alpha","alpha-helper"]},
            "beta 1.0.0 (path+file:///home/user/src/beta)":{"bins":["beta"]}
        }}"#;
        let installs = parse_crates2(text);
        assert_eq!(
            installs,
            vec![
                Install {
                    name: "alpha".into(),
                    version: "0.3.0".into(),
                    from: From::Git("https://example.com/alpha.git".into()),
                    bins: vec!["alpha".into(), "alpha-helper".into()],
                },
                Install {
                    name: "beta".into(),
                    version: "1.0.0".into(),
                    from: From::Path("/home/user/src/beta".into()),
                    bins: vec!["beta".into()],
                },
                Install {
                    name: "zeta-tool".into(),
                    version: "2.1.0".into(),
                    from: From::Registry("crates.io".into()),
                    bins: vec!["zt".into()],
                },
            ]
        );
    }

    #[test]
    fn runs_in_a_manifest_lists_the_programs_a_crate_starts() {
        let text = "[package]\nname = \"demo\"\ndescription = \"\"\"\na demo,\n  on two lines\"\"\"\n\n\
                    [package.metadata]\nruns = [\"ssh\", \"rsync\"]\n\n[dependencies]\n";
        assert_eq!(
            parse_manifest(text),
            Manifest {
                description: Some("a demo, on two lines".into()),
                runs: vec!["ssh".into(), "rsync".into()],
            }
        );
        assert_eq!(
            parse_manifest("[package]\nname = \"bare\"\n").runs,
            Vec::<String>::new(),
            "a crate without the table runs nothing"
        );
    }
}
