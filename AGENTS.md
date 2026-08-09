<!--
AI-ONLY DOCUMENT. This file exists to give an AI agent the COMPLETE operating picture for this repo. Optimize for completeness and precision for the agent, not for human readability. Humans read README.md instead. Do not remove detail to make this nicer — err toward more explicit, not less. FORMAT: machine-read, not a formatted human doc. Do NOT hard-wrap lines to a column width for readability; put each rule/point on ONE line, however long.
-->
# whypkg

## Hard rules
- **Commit, push, and publish only when the user says to ship**; never a mid-work checkpoint.
- Release flow, in this exact order: `cargo clippy` warning-clean + `cargo test` green -> bump `version` in `Cargo.toml` -> one commit (short conventional message, never co-authored) -> `git push origin main` -> `cargo publish` (dry-run first; publishing is irreversible) -> **tag only after publish succeeds**: `git tag vX.Y.Z && git push origin --tags`. A tag must never point at a version that failed to publish.
- Commit messages: short, single-line, conventional prefix (`feat:`/`fix:`/`chore:`). Never a `Co-Authored-By` trailer or a verbose body.
- **No em-dashes** anywhere user-facing (README, --help, crate description, commits); they read as AI-generated. Use `-`.
- **whypkg never syncs or modifies the system.** It only reads package state; a write would betray the whole premise ("just tell me why this is here").

## Invariants and gotchas
- Adding a distro is one new `src/backend/<name>.rs` (implement the `Backend` trait) plus one line in `detect()`; nothing else changes. Parsing in each backend is pure functions, tested against captured real output in `tests/fixtures/` via `include_str!`.
- Flatpak is NOT a `detect()` backend: it *coexists* with the system package manager, so `src/backend/flatpak.rs` layers apps onto whatever `World` the system backend built (`augment()`), each tagged `Source::Flatpak`. `load_world()` in `src/commands/mod.rs` calls it whenever `flatpak` is on PATH, and falls back to `World::empty()` if flatpak is the only thing present.
- Flatpak packages are keyed by application ID (`im.riot.Riot`) because that is what you type to remove one; the human name goes first in `description` ("Element - ..."), and the browser's fuzzy search matches name + description + `details`, which is the only reason such packages are findable by their real name. The same mechanism makes apt's `code` findable by "visual studio" (its long description).
- `Package.origin` (`Repo`/`Local`/`Orphaned`) answers "did I sideload this?": apt combines the `[installed,local]` tag from `apt list --installed` with a scan of `/var/log/apt/history.log*` for `.deb` installs; pacman uses `pacman -Qm` (foreign); dnf uses `from_repo` (`@commandline` = local, `@System` = orphaned). The local-vs-orphaned split is best-effort since history logs rotate.
- The graph view (`src/commands/graph.rs`, `Ctrl+G`) is a ratatui `Canvas` with braille markers, so it works in every terminal including tmux and over ssh - never reach for an image protocol (kitty/sixel) here, tmux breaks them. Nodes are spaced evenly *in y* (not by angle) so labels can never share a terminal row, on two alternating orbits so neighbouring labels start at different x; `POLE_FRAC` keeps the arc extremes off the centre line so trails do not run through each other. Labels sit on their planet's row and fan away from the middle. The layout tests encode all of this - if a change makes them fail, the visual bug is real.
- Rust ignores `SIGPIPE`, which made `whypkg pending --quick | head` panic; `restore_sigpipe()` in `main.rs` resets it to the default on unix. Never remove it, and keep report output going through plain `println!`.
- `capture()` forces `LC_ALL=C`, so labels, dates, and sizes parse regardless of the user's locale; never read tool output without it.
- When resolving dnf/rpm deps: they are *capabilities*, not package names, mapped to packages via a PROVIDES+FILENAMES provider map (`src/backend/dnf.rs`). pacman gives `Required By` + `Install Reason` natively.
- Upgradables reflect the user's last sync (`pacman -Qu`, `apt list --upgradable`, `dnf repoquery --upgrades --cacheonly`), so like `apt list --upgradable` they are only as fresh as the last `apt update`; whypkg must not sync to refresh them.
- When touching the browser: it needs a real TTY (errors cleanly otherwise). `Ctrl+J` collides with Enter and `Ctrl+I` with Tab unless the terminal speaks the kitty keyboard protocol (which `setup_terminal()` requests); arrows / `Ctrl+P` / `Ctrl+N` always work, so key hints must offer those. `Ctrl+[` is Esc at the byte level and is handled explicitly as such.
- To test the TUI without a TTY of your own, drive it in a detached tmux session (`tmux new-session -d -s x -x 150 -y 42 ./target/release/whypkg`, then `send-keys` and `capture-pane -p`); busy-poll `capture-pane` output for an expected string instead of sleeping. Visual judgement calls belong to the user, not the agent.
- Menu/footer items are separated by ` · `; the graph legend is not, because its `●` bullets already separate the entries.
- Testing other distros: build inside `archlinux`/`fedora`/`cachyos/cachyos` podman containers (`CARGO_TARGET_DIR=/tmp/...`) and regenerate fixtures from there.
- Mirror the `sluuz` crate's conventions: one clap-derive subcommand per file with `Args` + `run()`, and heavy why-focused doc comments.
- The local dir is `apt-why`, but the crate, repo, and remote are all `whypkg`.

## Build / test
- `cargo build` / `cargo build --release` (binary at `target/release/whypkg`).
- `cargo clippy` - keep warning-clean.
- `cargo test` - backend parsers checked against `tests/fixtures/`; `src/engine.rs` covers `bfs_root` (including cycle safety) and `src/commands/graph.rs` covers graph layout/navigation/history invariants. `World::from_edges()` (test-only, `src/model.rs`) builds synthetic worlds. When adding a layout/algorithm test, mutation-check it: reintroduce the bug and confirm the test actually fails, since a too-loose threshold silently passes.
- Run: `whypkg` (browse), `whypkg --upgradable`, `whypkg pending [--quick|--kernel|--apps|--auto|--sizes]`.
- README screenshots live in `readme-assets/` (excluded from the crate) and are referenced by absolute GitLab raw URLs, because relative paths do not render reliably on crates.io; they must be pushed before publishing or the URLs 404.

## Overview
`whypkg` ("why the hell is this package here?") is a fast, cross-distro Rust CLI that investigates installed packages: did you install it or did something pull it in, when, alongside what, what needs it, and what it needs. It is a rewrite of the original bash `apt-why`/`apt-pending` scripts (kept in `legacy/`, excluded from the published crate). Two modes share one distro-agnostic engine (`src/engine.rs`, with `bfs_root` as the "why is this here" origin trace): an interactive ratatui browser (default, with a `Ctrl+G` braille dependency-graph view) and a `pending` report classifying upgradable packages by why they are present. Backends: apt, pacman, dnf, plus flatpak layered on top of any of them. Crate and binary both `whypkg`, on crates.io, AGPL-3.0-only.

## Self-repair
If anything here contradicts the code, the code wins; fix AGENTS.md in the same session you notice the drift.
