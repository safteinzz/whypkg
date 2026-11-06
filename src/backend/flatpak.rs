//! Flatpak overlay.
//!
//! Unlike apt/dnf/pacman, Flatpak *coexists* with the system package manager,
//! so a machine has apt and flatpak at once. So this isn't a `detect()` backend
//! that wins; it reads its apps (and their runtimes) into a [`Layer`] that is
//! merged onto whatever `World` the system backend built, each tagged
//! `Source::Flatpak`.
//!
//! The app ID (`im.riot.Riot`) is cryptic, so we key packages by the ID but lead
//! the description with the friendly name (`Element - ...`). We also enrich each
//! app with its install date (the deploy directory's mtime), its remote (the
//! `origin` column, e.g. flathub), and a dependency edge to its runtime - so a
//! shared runtime shows "needed by: Brave, Discord, ..." when you open it.

use super::{Layer, capture, home, mtime};
use crate::model::{Origin, Package, Source};

/// Whether flatpak is installed. Cheap check so we don't shell out for nothing.
pub fn available() -> bool {
    super::have("flatpak")
}

/// Read installed flatpak apps and runtimes, each app with an edge to its
/// runtime. Two bulk `flatpak list` calls, tab-separated columns; a failed one
/// yields an empty layer.
pub fn read() -> Layer {
    let apps_raw = match capture(
        "flatpak",
        &[
            "list",
            "--app",
            "--columns=application,name,description,version,size,origin,runtime",
        ],
    ) {
        Ok(r) => r,
        Err(_) => return Layer::default(),
    };
    let (mut apps, edges) = parse_apps(&apps_raw);

    // Install date isn't a column; the deploy directory's mtime is a good proxy.
    for pkg in &mut apps {
        if let Some((epoch, date)) = deploy_date(&pkg.name) {
            pkg.install_epoch = Some(epoch);
            pkg.install_date = Some(date);
        }
    }

    let runtimes = capture(
        "flatpak",
        &[
            "list",
            "--runtime",
            "--columns=ref,name,version,size,origin",
        ],
    )
    .map(|r| parse_runtimes(&r))
    .unwrap_or_default();

    Layer {
        packages: apps.into_iter().chain(runtimes).collect(),
        deps: edges,
        needs: Vec::new(),
    }
}

/// Parse `flatpak list --app` (columns: application, name, description, version,
/// size, origin, runtime) into packages plus `(app, runtime_key)` edges.
pub fn parse_apps(text: &str) -> (Vec<Package>, Vec<(String, String)>) {
    let mut pkgs = Vec::new();
    let mut edges = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let app_id = match f.first() {
            Some(id) if !id.trim().is_empty() => id.trim().to_string(),
            _ => continue,
        };
        let friendly = f.get(1).map(|s| s.trim()).unwrap_or("");
        let blurb = f.get(2).map(|s| s.trim()).unwrap_or("");
        let version = f.get(3).map(|s| s.trim()).unwrap_or("");
        let size = f.get(4).map(|s| parse_size_kb(s.trim())).unwrap_or(0);
        let remote = f.get(5).map(|s| s.trim()).filter(|s| !s.is_empty());
        let runtime = f.get(6).map(|s| s.trim()).unwrap_or("");

        if !runtime.is_empty() {
            edges.push((app_id.clone(), runtime_key(runtime)));
        }

        pkgs.push(Package {
            name: app_id,
            label: None,
            version: version.to_string(),
            candidate: None,
            installed_size: size,
            description: describe(friendly, blurb),
            details: None,
            manual: true, // flatpak apps are always deliberately installed
            source: Source::Flatpak,
            remote: remote.map(String::from),
            origin: Origin::Repo,
            install_epoch: None,
            install_date: None,
        });
    }
    (pkgs, edges)
}

/// Parse `flatpak list --runtime` (columns: ref, name, version, size, origin)
/// into packages keyed by a short `id/branch` so app runtime refs resolve.
pub fn parse_runtimes(text: &str) -> Vec<Package> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            let key = match f.first() {
                Some(r) if !r.trim().is_empty() => runtime_key(r.trim()),
                _ => return None,
            };
            let friendly = f.get(1).map(|s| s.trim()).unwrap_or("");
            let version = f.get(2).map(|s| s.trim()).unwrap_or("");
            let size = f.get(3).map(|s| parse_size_kb(s.trim())).unwrap_or(0);
            let remote = f.get(4).map(|s| s.trim()).filter(|s| !s.is_empty());
            Some(Package {
                name: key,
                label: None,
                version: version.to_string(),
                candidate: None,
                installed_size: size,
                description: describe(friendly, "shared flatpak runtime"),
                details: None,
                manual: false, // runtimes are pulled in by the apps that use them
                source: Source::Flatpak,
                remote: remote.map(String::from),
                origin: Origin::Repo,
                install_epoch: None,
                install_date: None,
            })
        })
        .collect()
}

/// Lead a description with the friendly name, then a blurb.
fn describe(friendly: &str, blurb: &str) -> String {
    match (friendly.is_empty(), blurb.is_empty()) {
        (false, false) => format!("{friendly} - {blurb}"),
        (false, true) => friendly.to_string(),
        (true, false) => blurb.to_string(),
        (true, true) => String::new(),
    }
}

/// Normalise a flatpak ref/runtime string to a short `id/branch` key, dropping
/// the architecture: `org.freedesktop.Platform/x86_64/25.08` -> `.../25.08`.
/// Both an app's `runtime` column and a runtime's `ref` normalise the same way,
/// so edges resolve.
fn runtime_key(s: &str) -> String {
    let parts: Vec<&str> = s.trim_start_matches("runtime/").split('/').collect();
    if parts.len() >= 3 {
        format!("{}/{}", parts[0], parts[2]) // id/branch, drop arch
    } else {
        s.to_string()
    }
}

/// The install date proxy: the mtime of the app's deploy directory (system or
/// user install). Returns `(unix_epoch, "YYYY-MM-DD")`.
fn deploy_date(app_id: &str) -> Option<(i64, String)> {
    let system = std::path::PathBuf::from("/var/lib/flatpak/app").join(app_id);
    let user = home().map(|h| h.join(".local/share/flatpak/app").join(app_id));
    std::iter::once(system)
        .chain(user)
        .find_map(|path| mtime(&path))
}

/// Parse a flatpak human size like "544.4 MB" into KB (1024-based). Flatpak uses
/// a non-breaking space between number and unit, which `split_whitespace` still
/// treats as whitespace.
fn parse_size_kb(value: &str) -> u64 {
    let mut it = value.split_whitespace();
    let amount: f64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let mult = match it.next().unwrap_or("") {
        "B" | "bytes" => 1.0 / 1024.0,
        "kB" | "KB" | "KiB" => 1.0,
        "MB" | "MiB" => 1024.0,
        "GB" | "GiB" => 1024.0 * 1024.0,
        _ => 1.0,
    };
    (amount * mult) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_apps_with_friendly_names_and_runtime_edge() {
        let text = "im.riot.Riot\tElement\tSecure messenger\t1.12.24\t544.4 MB\tflathub\torg.freedesktop.Platform/x86_64/25.08\n\
                    org.gimp.GIMP\tGNU Image Manipulation Program\t\t3.2.4\t268.3 MB\tflathub\torg.gnome.Platform/x86_64/50\n";
        let (apps, edges) = parse_apps(text);
        assert_eq!(apps.len(), 2);

        let riot = &apps[0];
        assert_eq!(riot.name, "im.riot.Riot"); // keyed by app ID
        assert!(riot.description.starts_with("Element")); // friendly name first
        assert!(riot.manual);
        assert_eq!(riot.source, Source::Flatpak);
        assert_eq!(riot.remote.as_deref(), Some("flathub"));
        assert!(riot.installed_size > 0);

        // Edge points at the normalised runtime key (arch dropped).
        assert!(edges.contains(&(
            "im.riot.Riot".to_string(),
            "org.freedesktop.Platform/25.08".to_string()
        )));
    }

    #[test]
    fn runtime_ref_normalises_to_id_branch() {
        assert_eq!(
            runtime_key("org.freedesktop.Platform/x86_64/25.08"),
            "org.freedesktop.Platform/25.08"
        );
        assert_eq!(
            runtime_key("runtime/org.gnome.Platform/x86_64/50"),
            "org.gnome.Platform/50"
        );
    }

    #[test]
    fn runtimes_parse_as_non_manual() {
        let text = "org.freedesktop.Platform/x86_64/25.08\tFreedesktop Platform\t25.08\t900.0 MB\tflathub\n";
        let rts = parse_runtimes(text);
        assert_eq!(rts.len(), 1);
        assert_eq!(rts[0].name, "org.freedesktop.Platform/25.08");
        assert!(!rts[0].manual);
        assert_eq!(rts[0].source, Source::Flatpak);
    }

    #[test]
    fn size_units() {
        assert_eq!(parse_size_kb("2.0 MB"), 2048);
        assert_eq!(parse_size_kb("1.0 GB"), 1024 * 1024);
        assert_eq!(parse_size_kb("512 KiB"), 512);
    }
}
