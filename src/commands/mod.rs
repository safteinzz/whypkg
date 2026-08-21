pub mod browse;
pub mod pending;
pub mod selfcmd;

use crate::model::World;

/// Build the whole [`World`]: the system package manager's packages, plus any
/// Flatpak apps layered on top (the two coexist). Both the browser and the
/// report start here. On failure we print a clear message and exit.
pub fn load_world() -> World {
    let mut world = match crate::backend::detect() {
        Some(backend) => match backend.build_world() {
            Ok(world) => world,
            Err(e) => {
                eprintln!(
                    "whypkg: failed to read package data ({}): {e}",
                    backend.name()
                );
                std::process::exit(1);
            }
        },
        None => {
            // No system package manager. If Flatpak is present we can still be
            // useful; otherwise there's nothing to show.
            if crate::backend::flatpak::available() {
                World::empty()
            } else {
                eprintln!(
                    "whypkg: no supported package manager found.\n\
                     supported: apt/dpkg, pacman, dnf (and flatpak, alongside any of them)."
                );
                std::process::exit(1);
            }
        }
    };

    // Layer Flatpak apps on top of the system packages, if flatpak is installed.
    if crate::backend::flatpak::available() {
        crate::backend::flatpak::augment(&mut world);
    }

    world
}
