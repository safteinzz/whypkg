//! Distro-agnostic analysis over a [`World`]: the algorithms that turn raw
//! package data into answers. None of this knows or cares whether the data came
//! from apt, pacman, or dnf - that's the backend's job. This is the part that
//! was painful in bash (graph BFS, window heuristics) and is trivial here.

use crate::model::World;

/// How many reverse-deps make a package "foundational" (a core library many
/// things need) rather than something a user would consider removing.
pub const FOUNDATION_THRESHOLD: usize = 25;

/// Kernel / firmware / microcode packages: never "safe to remove", and grouped
/// specially in the report. Name-based, matching the original tool's rules.
pub fn is_kernel_pkg(name: &str) -> bool {
    name.starts_with("linux-")
        || name == "intel-microcode"
        || name == "amd64-microcode"
        || name.starts_with("firmware-")
        || name.starts_with("initramfs")
}

/// The headline answer to "why the hell is this here": walk the reverse-dep
/// graph outward from `start` until we reach a *manually* installed package,
/// and return the path `[start … manual_root]`. That root is the thing the user
/// actually chose to install; everything on the path was pulled in for it.
///
/// Returns `None` if no manual ancestor is found (an orphan / untraceable
/// auto-install). BFS guarantees the shortest such chain.
pub fn bfs_root(world: &World, start: &str) -> Option<Vec<String>> {
    use std::collections::{HashMap, HashSet, VecDeque};

    let mut visited: HashSet<&str> = HashSet::new();
    let mut parent: HashMap<&str, &str> = HashMap::new();
    let mut queue: VecDeque<&str> = VecDeque::new();

    visited.insert(start);
    queue.push_back(start);

    let mut found: Option<&str> = None;
    'search: while let Some(cur) = queue.pop_front() {
        for dep in world.rdeps_of(cur) {
            let dep = dep.as_str();
            if !visited.insert(dep) {
                continue;
            }
            parent.insert(dep, cur);
            if world.is_manual(dep) {
                found = Some(dep);
                break 'search;
            }
            queue.push_back(dep);
        }
    }

    let found = found?;

    // Reconstruct the path from the manual root back to start, then reverse it
    // so it reads start → … → root.
    let mut path = vec![found.to_string()];
    let mut node = found;
    while let Some(&p) = parent.get(node) {
        path.push(p.to_string());
        node = p;
    }
    path.reverse();
    Some(path)
}

/// Packages installed around the same time as `pkg` - a strong context clue for
/// "what did I install this alongside?". We find `pkg`'s install time, then
/// widen the window (3d → 1d → 12h → 6h) until it holds a manageable number of
/// packages, mirroring the original heuristic. Returns names, deduped & sorted.
pub fn same_session(world: &World, pkg: &str) -> Vec<String> {
    let anchor = match world.packages.get(pkg).and_then(|p| p.install_epoch) {
        Some(e) => e,
        None => return Vec::new(),
    };

    // Widest-first so we prefer more context, but fall back to tighter windows
    // if a busy install day would otherwise return hundreds of packages.
    const WINDOWS: [i64; 4] = [259_200, 86_400, 43_200, 21_600]; // 3d 1d 12h 6h

    let collect = |half: i64| -> Vec<String> {
        let (lo, hi) = (anchor - half, anchor + half);
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for (epoch, name) in &world.install_log {
            if *epoch < lo || *epoch > hi {
                continue;
            }
            if name == pkg {
                continue;
            }
            if seen.insert(name.clone()) {
                out.push(name.clone());
            }
        }
        out.sort();
        out
    };

    for half in WINDOWS {
        let result = collect(half);
        if result.len() <= 20 {
            return result;
        }
    }
    collect(21_600)
}

/// A rough "how long ago" string for a unix timestamp, e.g. "3 months ago".
/// Complements the absolute date rather than replacing it.
pub fn relative_time(epoch: i64) -> String {
    let secs = chrono::Utc::now().timestamp() - epoch;
    if secs < 0 {
        return "in the future".to_string();
    }
    let days = secs / 86_400;
    let plural = |n: i64| if n == 1 { "" } else { "s" };
    match days {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        2..=6 => format!("{days} days ago"),
        7..=29 => {
            let w = days / 7;
            format!("{w} week{} ago", plural(w))
        }
        30..=364 => {
            let m = days / 30;
            format!("{m} month{} ago", plural(m))
        }
        _ => {
            let y = days / 365;
            format!("{y} year{} ago", plural(y))
        }
    }
}

/// Format an installed size given in KB into a compact human string.
pub fn format_size(kb: u64) -> String {
    if kb == 0 {
        "n/a".to_string()
    } else if kb >= 1024 {
        format!("{:.1} MB", kb as f64 / 1024.0)
    } else {
        format!("{kb} KB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::World;

    // ── bfs_root: the "why is this here" origin trace ────────────────────────

    #[test]
    fn traces_an_auto_package_to_the_manual_one_that_pulled_it_in() {
        // firefox (you installed it) -> libnss3 -> libfoo
        let w = World::from_edges(
            &[("firefox", "libnss3"), ("libnss3", "libfoo")],
            &["firefox"],
        );
        let path = bfs_root(&w, "libfoo").expect("libfoo should trace to firefox");
        assert_eq!(path.first().unwrap(), "libfoo");
        assert_eq!(path.last().unwrap(), "firefox");
    }

    #[test]
    fn picks_the_shortest_chain_when_several_manual_roots_exist() {
        // libfoo is needed by libnss3 (-> firefox) and directly by vlc.
        let w = World::from_edges(
            &[
                ("firefox", "libnss3"),
                ("libnss3", "libfoo"),
                ("vlc", "libfoo"),
            ],
            &["firefox", "vlc"],
        );
        let path = bfs_root(&w, "libfoo").unwrap();
        assert_eq!(path.last().unwrap(), "vlc", "vlc is one hop away");
        assert_eq!(path.len(), 2);
    }

    #[test]
    fn returns_none_for_an_orphan_with_no_manual_ancestor() {
        let w = World::from_edges(&[("liba", "libb")], &[]);
        assert!(bfs_root(&w, "libb").is_none());
    }

    #[test]
    fn survives_dependency_cycles() {
        // Mutually-dependent packages are real (and a naive walk would hang).
        let w = World::from_edges(&[("liba", "libb"), ("libb", "liba")], &[]);
        assert!(bfs_root(&w, "liba").is_none());

        // …and still finds the root when a cycle sits in the middle.
        let w = World::from_edges(
            &[("app", "liba"), ("liba", "libb"), ("libb", "liba")],
            &["app"],
        );
        assert_eq!(bfs_root(&w, "libb").unwrap().last().unwrap(), "app");
    }

    #[test]
    fn a_manual_package_still_traces_out_to_whatever_needs_it() {
        // Being manual doesn't stop the walk at itself: `bfs_root` looks
        // *outward*, so a manual leaf with no dependents has no root.
        let w = World::from_edges(&[("app", "libfoo")], &["app", "libfoo"]);
        assert_eq!(bfs_root(&w, "libfoo").unwrap().last().unwrap(), "app");
        assert!(bfs_root(&w, "app").is_none());
    }

    // ── same_session ─────────────────────────────────────────────────────────

    #[test]
    fn same_session_is_empty_without_an_install_time() {
        let w = World::from_edges(&[("app", "lib")], &["app"]);
        assert!(same_session(&w, "app").is_empty());
    }

    #[test]
    fn same_session_returns_packages_installed_near_the_anchor() {
        let mut w = World::from_edges(&[("app", "lib")], &["app"]);
        let t = 1_700_000_000;
        w.packages.get_mut("app").unwrap().install_epoch = Some(t);
        w.install_log = vec![
            (t - 10, "installed-with-it".into()),
            (t, "app".into()),
            (t + 10, "also-with-it".into()),
            (t + 60 * 60 * 24 * 30, "much-later".into()),
        ];
        let session = same_session(&w, "app");
        assert!(session.contains(&"installed-with-it".to_string()));
        assert!(session.contains(&"also-with-it".to_string()));
        assert!(!session.contains(&"much-later".to_string()));
        assert!(!session.contains(&"app".to_string()), "excludes itself");
    }

    // ── classification helpers ───────────────────────────────────────────────

    #[test]
    fn kernel_packages_are_recognised() {
        assert!(is_kernel_pkg("linux-image-6.1"));
        assert!(is_kernel_pkg("intel-microcode"));
        assert!(is_kernel_pkg("firmware-realtek"));
        assert!(!is_kernel_pkg("firefox"));
        assert!(!is_kernel_pkg("linuxlogo"), "prefix must be `linux-`");
    }

    #[test]
    fn sizes_render_in_the_right_unit() {
        assert_eq!(format_size(0), "n/a");
        assert_eq!(format_size(512), "512 KB");
        assert_eq!(format_size(1024), "1.0 MB");
    }

    #[test]
    fn relative_time_reads_naturally() {
        let now = chrono::Utc::now().timestamp();
        assert_eq!(relative_time(now), "today");
        assert_eq!(relative_time(now - 86_400), "yesterday");
        assert_eq!(relative_time(now - 86_400 * 3), "3 days ago");
        assert_eq!(relative_time(now - 86_400 * 14), "2 weeks ago");
        assert_eq!(relative_time(now - 86_400 * 60), "2 months ago");
        assert_eq!(relative_time(now - 86_400 * 400), "1 year ago");
    }
}
