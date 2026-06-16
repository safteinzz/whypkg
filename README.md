# whypkg

**why the hell is this package here?** 🕵️

A fast, cross-distro package investigator. Fuzzy-find any installed package and
see, instantly: did *you* install it or did something pull it in, when, what it
came alongside, what needs it, and what it needs — then drill through the whole
dependency web, following the thread inward and outward.

It started life as the bash `apt-why` / `apt-pending` scripts (kept in
[`legacy/`](legacy/)); this is the Rust rewrite — one self-contained binary, no
external `fzf`, and built to grow beyond apt.

## Install

```bash
cargo install whypkg
```

Update later with `whypkg update` (or `cargo install whypkg --force`).

## Use

```bash
whypkg                 # browse every installed package
whypkg --upgradable    # browse only packages with a pending upgrade
whypkg pending         # report every pending upgrade, grouped by why it's here
whypkg pending --quick # one line per pending package: size + reason
```

Inside the browser: **type** to fuzzy-filter, **Enter** to open a package's
dossier, **Esc** to go back a level (a breadcrumb shows your trail), **Ctrl-C**
to quit. `[M]` = you installed it, `[A]` = pulled in automatically, `↑` = an
upgrade is available.

## Why it's fast

Everything is loaded once at startup into an in-memory graph; every hop while
you browse is a hash-map lookup, not a subprocess. The slowest part is your
package manager's own queries (~½ second), not whypkg.

## Distro support

| Distro family         | Status     |
|-----------------------|------------|
| Debian / Ubuntu (apt) | ✅ working  |
| Arch (pacman)         | ✅ working  |
| Fedora / RHEL (dnf)   | ✅ working  |

Each package manager lives behind a single `Backend` trait — the analysis and
the UI are distro-agnostic, so adding pacman or dnf is one focused file.

## License

AGPL-3.0-only.
