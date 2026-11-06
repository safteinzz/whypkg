//! pipx and uv tools: each one is a virtualenv holding the tool and the Python
//! packages it pulled in.
//!
//! Everything is read from the files the installers leave behind, never by
//! running them, since each costs a Python or a process start. A tool is the
//! package you asked for, every other distribution in its virtualenv is pulled
//! in by it (or asked for alongside it, `--with` or `inject`), and the Python
//! it runs on is a `uses` edge to the system package that owns it.

use super::{Layer, data_home, home, mtime};
use crate::model::{Origin, Package, Source};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Every pipx tool, under `$PIPX_HOME/venvs` (`~/.local/share/pipx`, or the
/// older `~/.local/pipx`).
pub fn read_pipx() -> Layer {
    let homes: Vec<PathBuf> = match std::env::var_os("PIPX_HOME").filter(|h| !h.is_empty()) {
        Some(h) => vec![PathBuf::from(h)],
        None => data_home()
            .map(|d| d.join("pipx"))
            .into_iter()
            .chain(home().map(|h| h.join(".local/pipx")))
            .collect(),
    };
    let mut layer = Layer::default();
    // The older home can still hold a tool pipx has since moved; the newer
    // copy, read first, is the one pipx runs.
    let mut read: HashSet<std::ffi::OsString> = HashSet::new();
    for venvs in homes.iter().map(|h| h.join("venvs")) {
        for dir in subdirs(&venvs) {
            if !dir.file_name().is_some_and(|n| read.insert(n.to_owned())) {
                continue;
            }
            let meta = dir.join("pipx_metadata.json");
            let (main, asked) = std::fs::read_to_string(&meta)
                .map(|t| parse_pipx_metadata(&t))
                .unwrap_or_default();
            read_tool(Source::Pipx, &dir, &meta, main, &asked, &mut layer);
        }
    }
    layer
}

/// Every uv tool, under `$UV_TOOL_DIR` (`~/.local/share/uv/tools`).
pub fn read_uv() -> Layer {
    let root = std::env::var_os("UV_TOOL_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| data_home().map(|d| d.join("uv/tools")));
    let mut layer = Layer::default();
    for dir in root.iter().flat_map(|r| subdirs(r)) {
        let receipt = dir.join("uv-receipt.toml");
        let asked = std::fs::read_to_string(&receipt)
            .map(|t| parse_uv_receipt(&t))
            .unwrap_or_default();
        let main = asked.first().cloned();
        read_tool(Source::Uv, &dir, &receipt, main, &asked, &mut layer);
    }
    layer
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// One distribution in a virtualenv, from its `.dist-info`.
#[derive(Debug, Default, PartialEq)]
pub struct Dist {
    /// The name as the project writes it (`PyYAML`).
    pub name: String,
    pub version: String,
    pub summary: String,
    /// Normalized names of what it requires, extras and markers included: a
    /// requirement whose distribution is not in the virtualenv is dropped
    /// later, which is what a marker for another platform comes to.
    pub requires: Vec<String>,
}

/// Read the tool in `dir` into `layer`. `main` is the distribution the tool
/// was installed as, `asked` every one the user named, and `stamp` the file
/// whose mtime dates the install.
fn read_tool(
    source: Source,
    dir: &Path,
    stamp: &Path,
    main: Option<String>,
    asked: &[String],
    layer: &mut Layer,
) {
    let Some(tool) = dir.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let Some(site) = site_packages(dir) else {
        return;
    };
    let main = normalize(main.as_deref().unwrap_or(tool));
    let asked: HashSet<String> = asked.iter().map(|a| normalize(a)).collect();
    let date = mtime(stamp).or_else(|| mtime(dir));

    // Every distribution, keyed by its normalized name.
    let mut dists: HashMap<String, (Dist, PathBuf)> = HashMap::new();
    for info in std::fs::read_dir(&site)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
    {
        let path = info.path();
        if path.extension().and_then(|e| e.to_str()) != Some("dist-info") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path.join("METADATA")) else {
            continue;
        };
        let dist = parse_metadata(&text);
        if !dist.name.is_empty() {
            dists.insert(normalize(&dist.name), (dist, path));
        }
    }
    if !dists.contains_key(&main) {
        return;
    }

    let prefix = source.word();
    let key = |norm: &str| {
        if *norm == main {
            format!("{prefix}:{tool}")
        } else {
            format!("{prefix}:{tool}/{norm}")
        }
    };
    let tool_key = key(&main);
    for (norm, (dist, info)) in &dists {
        let is_main = *norm == main;
        let requested = is_main || asked.contains(norm) || info.join("REQUESTED").exists();
        let local = direct_url_path(info);
        layer.packages.push(Package {
            name: key(norm),
            label: Some(dist.name.clone()),
            version: dist.version.clone(),
            candidate: None,
            installed_size: record_size(&info.join("RECORD")),
            description: dist.summary.clone(),
            details: None,
            manual: requested,
            source,
            remote: Some(local.clone().unwrap_or_else(|| "PyPI".to_string())),
            origin: if local.is_some() {
                Origin::Local
            } else {
                Origin::Repo
            },
            install_epoch: date.as_ref().map(|(epoch, _)| *epoch),
            install_date: date.as_ref().map(|(_, day)| day.clone()),
        });
        for req in &dist.requires {
            if dists.contains_key(req) && req != norm {
                layer.deps.push((key(norm), key(req)));
            }
        }
    }
    if let Ok(python) = dir.join("bin/python").canonicalize() {
        layer.needs.push((tool_key, python));
    }
}

/// `lib/python3.X/site-packages` inside a virtualenv.
fn site_packages(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir.join("lib"))
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path().join("site-packages"))
        .find(|p| p.is_dir())
}

/// The local path a distribution was installed from (`pipx install ./x`,
/// `uv tool install --editable .`), from its `direct_url.json`.
fn direct_url_path(info: &Path) -> Option<String> {
    let text = std::fs::read_to_string(info.join("direct_url.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let url = json.get("url")?.as_str()?;
    url.strip_prefix("file://").map(String::from)
}

/// The size of a distribution in KB, summed from its `RECORD`
/// (`path,hash,size`), so no file has to be stat'ed.
fn record_size(record: &Path) -> u64 {
    std::fs::read_to_string(record)
        .map(|text| {
            text.lines()
                .filter_map(|l| l.rsplit(',').next()?.trim().parse::<u64>().ok())
                .sum::<u64>()
                / 1024
        })
        .unwrap_or(0)
}

/// Parse the headers of a `METADATA` file, which end at the first blank line.
pub fn parse_metadata(text: &str) -> Dist {
    let mut dist = Dist::default();
    for line in text.lines() {
        if line.is_empty() {
            break;
        }
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match field {
            "Name" => dist.name = value.to_string(),
            "Version" => dist.version = value.to_string(),
            "Summary" => dist.summary = value.to_string(),
            "Requires-Dist" => {
                let name: String = value
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                    .collect();
                if !name.is_empty() {
                    dist.requires.push(normalize(&name));
                }
            }
            _ => {}
        }
    }
    dist
}

/// A distribution name the way pip compares them: lowercase, with every run
/// of `.`, `_` and `-` as one `-` (`PyYAML` and `pyyaml` are one project).
pub fn normalize(name: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in name.chars() {
        if matches!(c, '.' | '_' | '-') {
            gap = true;
            continue;
        }
        if gap && !out.is_empty() {
            out.push('-');
        }
        gap = false;
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// `(main package, every package named)` from `pipx_metadata.json`: the
/// package the venv was made for, and the ones injected into it.
pub fn parse_pipx_metadata(text: &str) -> (Option<String>, Vec<String>) {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return (None, Vec::new());
    };
    let main = json
        .pointer("/main_package/package")
        .and_then(|p| p.as_str())
        .map(String::from);
    let injected = json
        .get("injected_packages")
        .and_then(|i| i.as_object())
        .map(|i| i.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let asked = main.iter().cloned().chain(injected).collect();
    (main, asked)
}

/// Every package named in a `uv-receipt.toml`, in order: the tool first, then
/// what `--with` added.
pub fn parse_uv_receipt(text: &str) -> Vec<String> {
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return Vec::new();
    };
    doc.get("tool")
        .and_then(|t| t.get("requirements"))
        .and_then(|r| r.as_array())
        .map(|reqs| {
            reqs.iter()
                .filter_map(|r| r.as_inline_table()?.get("name")?.as_str())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A virtualenv under a temp folder the test deletes.
    struct Venv(PathBuf);

    impl Venv {
        fn new(tool: &str) -> Venv {
            let root =
                std::env::temp_dir().join(format!("whypkg-venv-{}-{tool}", std::process::id()));
            std::fs::create_dir_all(root.join(tool).join("lib/python3.13/site-packages")).unwrap();
            Venv(root)
        }

        fn dist(&self, tool: &str, name: &str, requires: &[&str], requested: bool) {
            let info = self
                .0
                .join(tool)
                .join("lib/python3.13/site-packages")
                .join(format!("{name}-1.0.dist-info"));
            std::fs::create_dir_all(&info).unwrap();
            let mut meta = format!("Metadata-Version: 2.1\nName: {name}\nVersion: 1.0\n");
            for r in requires {
                meta.push_str(&format!("Requires-Dist: {r}\n"));
            }
            meta.push('\n');
            std::fs::write(info.join("METADATA"), meta).unwrap();
            if requested {
                std::fs::write(info.join("REQUESTED"), "").unwrap();
            }
        }
    }

    impl Drop for Venv {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn requires_dist_links_what_is_in_the_virtualenv_by_pip_names() {
        let v = Venv::new("httpie");
        v.dist(
            "httpie",
            "httpie",
            &[
                "Pygments>=2.5",
                "requests[socks]",
                "pywin32; sys_platform == 'win32'",
            ],
            false,
        );
        v.dist("httpie", "pygments", &[], false);
        v.dist("httpie", "Requests", &[], false);
        let mut layer = Layer::default();
        let dir = v.0.join("httpie");
        read_tool(
            Source::Pipx,
            &dir,
            &dir,
            Some("httpie".into()),
            &[],
            &mut layer,
        );
        let mut deps = layer.deps.clone();
        deps.sort();
        assert_eq!(
            deps,
            vec![
                (
                    "pipx:httpie".to_string(),
                    "pipx:httpie/pygments".to_string()
                ),
                (
                    "pipx:httpie".to_string(),
                    "pipx:httpie/requests".to_string()
                ),
            ],
            "a requirement not in the virtualenv (pywin32) must not become an edge"
        );
        assert_eq!(normalize("Py_YAML.x"), "py-yaml-x");
    }

    #[test]
    fn a_tool_and_what_was_asked_for_count_as_yours() {
        let v = Venv::new("black");
        v.dist("black", "black", &["click"], false);
        v.dist("black", "click", &[], false);
        v.dist("black", "rich", &[], true);
        v.dist("black", "isort", &[], false);
        let mut layer = Layer::default();
        let dir = v.0.join("black");
        read_tool(
            Source::Uv,
            &dir,
            &dir,
            Some("black".into()),
            &["black".into(), "isort".into()],
            &mut layer,
        );
        let manual = |key: &str| {
            layer
                .packages
                .iter()
                .find(|p| p.name == key)
                .map(|p| p.manual)
        };
        assert_eq!(manual("uv:black"), Some(true), "the tool itself");
        assert_eq!(manual("uv:black/rich"), Some(true), "marked REQUESTED");
        assert_eq!(manual("uv:black/isort"), Some(true), "named in the receipt");
        assert_eq!(manual("uv:black/click"), Some(false), "only pulled in");
    }

    #[test]
    fn the_receipts_name_the_tool_and_what_was_added_to_it() {
        let pipx =
            r#"{"main_package":{"package":"httpie"},"injected_packages":{"httpie-plugin":{}}}"#;
        assert_eq!(
            parse_pipx_metadata(pipx),
            (
                Some("httpie".to_string()),
                vec!["httpie".to_string(), "httpie-plugin".to_string()]
            )
        );
        let uv = "[tool]\nrequirements = [{ name = \"black\" }, { name = \"isort\", specifier = \">=5\" }]\n";
        assert_eq!(parse_uv_receipt(uv), vec!["black", "isort"]);
    }
}
