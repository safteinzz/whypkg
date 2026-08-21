<!--
AI-ONLY DOCUMENT. This file exists to give an AI agent the COMPLETE operating picture for this repo. Optimize for completeness and precision for the agent, not for human readability. Humans read README.md instead. Do not remove detail to make this nicer, err toward more explicit, not less. FORMAT: machine-read, not a formatted human doc. Do NOT hard-wrap lines to a column width for readability; put each rule/point on ONE line, however long.
-->
# AGENTS.md

Working brief for an AI coding agent, not documentation for people (the README covers that): the rules, invariants and gotchas needed to change this project correctly without rediscovering them.

## Hard rules
- Commit, push, and publish only when the user says to ship; a mid-work commit is never the deliverable, because the user tests interactively first.
- Commit messages are short single-line conventional ones (`feat:`, `fix:`, `chore:`, ...), never with a `Co-Authored-By` trailer and never with a verbose body.
- Release flow, in this exact order: ask whether this shipment gets tests and write them only if the user says yes -> bump `version` in `Cargo.toml` -> `cargo clippy-all` clean and `cargo test` green, which is also what refreshes `Cargo.lock` with the new version -> one commit -> `git push origin main` -> `cargo publish` (dry-run first, publishing is irreversible) -> tag only after publish succeeds with `git tag vX.Y.Z && git push origin --tags`; a tag must never point at a version that failed to publish, and the bump comes first because `cargo publish` fails on a `Cargo.lock` that still holds the old version.
- Tests are proposed at ship time and never before: the first step of the release flow is to ask the user, in plain words, whether this shipment gets tests, and they are written only on a yes, so the decision is always theirs but the question is never forgotten.
- Never write a test for behaviour that has not shipped yet, because code that is not in the last release tag is still being designed, and a test pinning a shape that is about to change is how a suite starts lying.
- A test may only assert something the README or `--help` promises, or a pure-logic invariant (parsing, generation, path resolution, validation); never the shape of a private function and never the specific diff that was just made, since those rot on the next refactor and teach nothing about whether the program works.
- Removing a promise from the README removes its tests in the same commit.
- A test may only write inside a temp directory it deletes, never a real config, data, cache or content directory and never a fixed path, so a machine is left exactly as it was before the suite ran.
- Never drive the interface to test it: build it, say what changed and what to look at, and let the user run it, because they see the screen instantly while an agent driving a pty or a tmux pane is slow and wrong about what it looks like; logic that is not visual can still be checked directly from `tests/`.
- Never `cargo install` to test: run the release binary at `./target/release/whypkg` directly, because installing replaces the binary on PATH with a work-in-progress build; install only when the user asks.
- `main` is protected: no force-push and no history rewrite, so a mistake is fixed with a forward commit.
- No em-dashes anywhere (code, comments, README, `--help`, crate description, commit messages, prose), because they read as AI-generated text; use `-` instead.
- Fix the root cause, and if a workaround must ship say the word "workaround" out loud so a silent patch never passes as a real fix; the same goes for lints, where an `#[allow]` is never the answer and the code it points at gets fixed or deleted.
- `TODO-LIST.md` (gitignored) holds one-line ideas, and the line is deleted when the idea ships.
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
- Menu/footer items are separated by ` · `; the graph legend is not, because its `●` bullets already separate the entries.
- Testing other distros: build inside `archlinux`/`fedora`/`cachyos/cachyos` podman containers (`CARGO_TARGET_DIR=/tmp/...`) and regenerate fixtures from there.
- Mirror the `sluuz` crate's conventions: one clap-derive subcommand per file with `Args` + `run()`, and heavy why-focused doc comments.
- The local dir is `apt-why`, but the crate, repo, and remote are all `whypkg`.

## Build / lint / test
- `cargo build --release`, binary at `target/release/whypkg`.
- `cargo clippy-all` is the lint pass, aliased in `.cargo/config.toml` to `clippy --release --all-targets -- -D warnings`; use it rather than a bare `cargo clippy`, which skips `tests/` and `examples/` and only warns where the release flow wants a failure.
- `cargo test`.
- Testing other distros means building inside `archlinux`/`fedora`/`cachyos/cachyos` podman containers (`CARGO_TARGET_DIR=/tmp/...`) and regenerating fixtures from there.
- When adding a layout or algorithm test, mutation-check it by reintroducing the bug and confirming the test actually fails, since a too-loose threshold silently passes.
- Backend parsers are checked against captured real output in `tests/fixtures/` via `include_str!`, `src/engine.rs` covers `bfs_root` including cycle safety, and `src/commands/graph.rs` covers graph layout, navigation and history invariants, with `World::from_edges()` (test-only, `src/model.rs`) building synthetic worlds.
- Run: `whypkg` (browse), `whypkg --upgradable`, `whypkg pending [--quick|--kernel|--apps|--auto|--sizes]`.
- README screenshots live in `readme-assets/` (excluded from the crate) and are referenced by absolute GitLab raw URLs, because relative paths do not render reliably on crates.io; they must be pushed before publishing or the URLs 404.

## Overview
`whypkg` ("wonder why the f* you have that package? know it now") is a fast, cross-distro Rust CLI that investigates installed packages: did you install it or did something pull it in, when, alongside what, what needs it, and what it needs. It is a rewrite of the original bash `apt-why`/`apt-pending` scripts (kept in `legacy/`, excluded from the published crate). Two modes share one distro-agnostic engine (`src/engine.rs`, with `bfs_root` as the "why is this here" origin trace): an interactive ratatui browser (default, with a `Ctrl+G` braille dependency-graph view) and a `pending` report classifying upgradable packages by why they are present. Backends: apt, pacman, dnf, plus flatpak layered on top of any of them. Crate and binary both `whypkg`, on crates.io, AGPL-3.0-only.

## Self-repair
If anything here contradicts the code, the code wins; fix AGENTS.md in the same session you notice the drift.
