# whypkg

## Hard rules
- **Commit, push, and publish only when the user says to ship**; never a mid-work checkpoint.
- Release flow, in this exact order: `cargo clippy` warning-clean + `cargo test` green -> bump `version` in `Cargo.toml` -> one commit (short conventional message, never co-authored) -> `git push origin main` -> `cargo publish` (dry-run first; publishing is irreversible) -> **tag only after publish succeeds**: `git tag vX.Y.Z && git push origin --tags`. A tag must never point at a version that failed to publish.
- Commit messages: short, single-line, conventional prefix (`feat:`/`fix:`/`chore:`). Never a `Co-Authored-By` trailer or a verbose body.
- **No em-dashes** anywhere user-facing (README, --help, crate description, commits); they read as AI-generated. Use `-`.
- **whypkg never syncs or modifies the system.** It only reads package state; a write would betray the whole premise ("just tell me why this is here").

## Invariants and gotchas
- Adding a distro is one new `src/backend/<name>.rs` (implement the `Backend` trait) plus one line in `detect()`; nothing else changes. Parsing in each backend is pure functions, tested against captured real output in `tests/fixtures/` via `include_str!`.
- `capture()` forces `LC_ALL=C`, so labels, dates, and sizes parse regardless of the user's locale; never read tool output without it.
- When resolving dnf/rpm deps: they are *capabilities*, not package names, mapped to packages via a PROVIDES+FILENAMES provider map (`src/backend/dnf.rs`). pacman gives `Required By` + `Install Reason` natively.
- Upgradables reflect the user's last sync (`pacman -Qu`, `apt list --upgradable`, `dnf repoquery --upgrades --cacheonly`), so like `apt list --upgradable` they are only as fresh as the last `apt update`; whypkg must not sync to refresh them.
- When touching the browser: it needs a real TTY (errors cleanly otherwise). `Ctrl+J` collides with Enter unless the terminal speaks the kitty keyboard protocol; arrows / `Ctrl+P` / `Ctrl+N` always work, so key hints must offer those.
- Testing other distros: build inside `archlinux`/`fedora`/`cachyos/cachyos` podman containers (`CARGO_TARGET_DIR=/tmp/...`) and regenerate fixtures from there.
- Mirror the `sluuz` crate's conventions: one clap-derive subcommand per file with `Args` + `run()`, and heavy why-focused doc comments.
- The local dir is `apt-why`, but the crate, repo, and remote are all `whypkg`.

## Build / test
- `cargo build` / `cargo build --release` (binary at `target/release/whypkg`).
- `cargo clippy` - keep warning-clean.
- `cargo test` - backend parsers checked against `tests/fixtures/`.
- Run: `whypkg` (browse), `whypkg --upgradable`, `whypkg pending [--quick|--kernel|--apps|--auto|--sizes]`.

## Overview
`whypkg` ("why the hell is this package here?") is a fast, cross-distro Rust CLI that investigates installed packages: did you install it or did something pull it in, when, alongside what, what needs it, and what it needs. It is a rewrite of the original bash `apt-why`/`apt-pending` scripts (kept in `legacy/`, excluded from the published crate). Two modes share one distro-agnostic engine (`src/engine.rs`, with `bfs_root` as the "why is this here" origin trace): an interactive ratatui browser (default) and a `pending` report classifying upgradable packages by why they are present. Crate and binary both `whypkg`, on crates.io, AGPL-3.0-only.

## Self-repair
If anything here contradicts the code, the code wins; fix AGENTS.md in the same session you notice the drift.
