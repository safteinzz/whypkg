//! Reading the machine into one [`World`]: the system package manager and
//! every installer beside it, all at once, each on a thread of its own.
//!
//! The system backend is the slow one (about half a second of package-manager
//! queries), so the other sources and the one `owners` query that ties them to
//! system packages run while it does, and cost nothing on top.

use crate::backend::{self, Backend, Layer, elf::Libraries};
use crate::model::{Source, World};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;

/// The sources one run reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Sources {
    on: Vec<Source>,
}

impl Sources {
    /// Every source but the ones `skip` names.
    pub fn all_but(skip: &[Source]) -> Sources {
        Sources {
            on: Source::ALL
                .into_iter()
                .filter(|s| !skip.contains(s))
                .collect(),
        }
    }

    pub fn only(on: &[Source]) -> Sources {
        Sources {
            on: Source::ALL.into_iter().filter(|s| on.contains(s)).collect(),
        }
    }

    pub fn has(&self, source: Source) -> bool {
        self.on.contains(&source)
    }

    pub fn set(&mut self, source: Source, on: bool) {
        self.on.retain(|s| *s != source);
        if on {
            self.on.push(source);
        }
    }
}

/// Which readers are still going, for the rows drawn while the user waits.
#[derive(Default)]
pub struct Progress {
    readers: Mutex<Vec<(String, bool)>>,
}

impl Progress {
    fn start(&self, name: &str) {
        if let Ok(mut r) = self.readers.lock() {
            r.push((name.to_string(), false));
        }
    }

    fn finish(&self, name: &str) {
        if let Ok(mut r) = self.readers.lock()
            && let Some(entry) = r.iter_mut().find(|(n, _)| n == name)
        {
            entry.1 = true;
        }
    }

    /// `(still reading, finished, all started)`.
    pub fn snapshot(&self) -> (Vec<String>, usize, usize) {
        let Ok(r) = self.readers.lock() else {
            return (Vec::new(), 0, 0);
        };
        let pending = r
            .iter()
            .filter(|(_, done)| !done)
            .map(|(n, _)| n.clone())
            .collect();
        let done = r.iter().filter(|(_, done)| *done).count();
        (pending, done, r.len())
    }
}

/// Read every source in `sources` into one world. Fails only when the system
/// package manager's own queries fail, or when there is nothing to read at
/// all; a source beside it that cannot be read is left out.
pub fn read_world(sources: &Sources, progress: &Progress) -> Result<World, String> {
    // Detected even when the system's packages are not read, because npm needs
    // it to tell its own globals from node modules the distro packaged.
    let backend = backend::detect();
    let backend = backend.as_deref();
    let system_on = sources.has(Source::System);
    let (system, layers, owners) = thread::scope(|scope| {
        let system = backend.filter(|_| system_on).map(|b| {
            progress.start(b.name());
            scope.spawn(move || {
                let world = b.build_world().map_err(|e| (b.name(), e));
                progress.finish(b.name());
                world
            })
        });
        let (layers, owners) = read_layers(sources, backend, progress);
        let system = system.map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)));
        (system, layers, owners)
    });

    let mut world = match system {
        Some(Ok(world)) => world,
        Some(Err((name, e))) => return Err(format!("could not read {name}'s package data: {e}")),
        None => World::empty(),
    };
    world.manager = backend.filter(|_| system_on).map(|b| b.name());
    let mut needs = Vec::new();
    for layer in layers {
        needs.extend(layer.needs.iter().cloned());
        layer.merge_into(&mut world);
    }
    for (user, file) in needs {
        if let Some(owner) = owners.get(&file)
            && world.packages.contains_key(owner)
            && world.packages.contains_key(&user)
        {
            world.add_use(&user, owner);
        }
    }
    if world.packages.is_empty() && backend.is_none() && system_on {
        let others: Vec<&str> = Source::ALL[1..]
            .iter()
            .filter(|s| sources.has(**s))
            .map(|s| s.word())
            .collect();
        let mut said =
            "no supported package manager found: whypkg reads apt, pacman, dnf or zypper"
                .to_string();
        if !others.is_empty() {
            said.push_str(&format!(
                ", and found nothing installed through {}",
                others.join(", ")
            ));
        }
        return Err(said);
    }
    Ok(world)
}

/// Read every source beside the system package manager, each on its own
/// thread, then ask `backend` once who owns the files they need.
fn read_layers(
    sources: &Sources,
    backend: Option<&dyn Backend>,
    progress: &Progress,
) -> (Vec<Layer>, HashMap<PathBuf, String>) {
    let mut readers: Vec<(Source, Box<dyn FnOnce() -> Layer + Send + '_>)> = Vec::new();
    if sources.has(Source::Flatpak) && backend::flatpak::available() {
        readers.push((Source::Flatpak, Box::new(backend::flatpak::read)));
    }
    if sources.has(Source::Cargo) {
        readers.push((
            Source::Cargo,
            Box::new(|| backend::cargo::read(&Libraries::load())),
        ));
    }
    if sources.has(Source::Pipx) {
        readers.push((Source::Pipx, Box::new(backend::venv::read_pipx)));
    }
    if sources.has(Source::Uv) {
        readers.push((Source::Uv, Box::new(backend::venv::read_uv)));
    }
    if sources.has(Source::Npm) {
        readers.push((Source::Npm, Box::new(move || backend::npm::read(backend))));
    }

    let layers: Vec<Layer> = thread::scope(|scope| {
        let handles: Vec<_> = readers
            .into_iter()
            .map(|(source, read)| {
                progress.start(source.word());
                scope.spawn(move || {
                    let layer = read();
                    progress.finish(source.word());
                    layer
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });

    let files: Vec<PathBuf> = layers
        .iter()
        .flat_map(|l| l.needs.iter().map(|(_, f)| f.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    // The owners are system packages, which are only there to link to when
    // the system's packages were read.
    let owners = match backend {
        Some(b) if !files.is_empty() && sources.has(Source::System) => b.owners(&files),
        _ => HashMap::new(),
    };
    (layers, owners)
}
