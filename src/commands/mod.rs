pub mod browse;
pub mod pending;
pub mod update;

use crate::model::World;

/// Detect the system's package manager, build the whole [`World`], and hand it
/// back. Both the browser and the report start here. On failure we print a
/// clear message and exit — there's nothing useful to do without package data.
pub fn load_world() -> World {
    let backend = match crate::backend::detect() {
        Some(b) => b,
        None => {
            eprintln!(
                "whypkg: no supported package manager found.\n\
                 supported: apt/dpkg (pacman and dnf are planned)."
            );
            std::process::exit(1);
        }
    };

    match backend.build_world() {
        Ok(world) => world,
        Err(e) => {
            eprintln!(
                "whypkg: failed to read package data ({}): {e}",
                backend.name()
            );
            std::process::exit(1);
        }
    }
}
