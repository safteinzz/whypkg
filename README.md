# whypkg

> **Canonical:** [gitlab.com/safteinzz/whypkg](https://gitlab.com/safteinzz/whypkg) · **Mirror:** [github.com/safteinzz/whypkg](https://github.com/safteinzz/whypkg)

<!-- desc:start -->
why is that package here? know it now - a fast package investigator for your distro, flatpak, cargo, pipx, uv and npm
<!-- desc:end -->

## Install

```bash
cargo install whypkg
whypkg self check   # is a newer release out?
whypkg self update  # install the latest
```

No cargo yet? Rust installs the same way on every distro: [rustup.rs](https://rustup.rs).

## What have I actually got?

![The whypkg browser: every installed package in one list, with a tab for each installer: system, flatpak, cargo, pipx, uv and npm](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/browse.png)

Legend: `●` you installed it · `○` something pulled it in · `← clang` pulled in by clang · `↑` upgrade waiting

```bash
whypkg                 # browse every installed package
```

What your distro installed, and what flatpak, cargo, pipx, uv and npm did beside it, each in a tab of its own, and `tab` steps through them.

## What was that thing called?

![Typing /Element narrows the list to five packages, the flatpak app im.riot.Riot first](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/search.png)

`/` filters by descriptions as well as names, which is the only way anyone finds
a flatpak app by the name on its window.

## Why is this here?

![A package dossier: libllvm21, 133 MB, pulled in by clang, with the eleven packages that need it beside the nine it needs](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/dossier.png)

**pulled in by clang.** That is the whole point. You also get where it came
from, what needs it and what it needs; `↵` on anything in the lists follows the
thread and `esc` walks back.

## What breaks if I remove it?

![The nodejs dossier: three npm globals run on it, so removing it breaks them](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/uses.png)

A cargo binary loads system libraries, and a pipx, uv or npm tool runs on the
system's Python or node, but no package manager records either. whypkg reads
both, so the package apt would happily remove names what still needs it.

## What is around it?

![The dependency graph view: libllvm21 in the centre, packages that need it on the left, packages it needs on the right](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/graph.png)

`ctrl-g` draws the neighbourhood in the terminal, with no browser and no image
protocol, so it survives ssh and tmux. `↵` re-centres on a neighbour and `esc`
retraces.

## What is this upgrade about to pull down?

![The pending report: one line per upgradable package with its size and why it is installed](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/pending.png)

```bash
whypkg --upgradable    # browse only packages with an upgrade waiting
whypkg pending         # full report, grouped by what pulled things in
whypkg pending --quick # one line per package: size + reason
```

Every package with an upgrade waiting, and why it is on your machine. Pipe it,
grep it, diff it before and after.

## Commands

```bash
whypkg why libnss3 jq          # the dossier as text, for any number of packages
whypkg why ripgrep             # every installer's copy; cargo:ripgrep for that one
whypkg leaves                  # what autoremove would take, sideloaded and orphaned packages
whypkg pending --kernel        # one section at a time; --apps, --auto and --sizes too
whypkg --only system,cargo     # read only some sources; why and leaves take it too
```

`whypkg <command> --help` has the details, and `?` in the browser lists every key.

Nothing prompts. `--quick` prints one line per package for a pipe, `--json` on
`why`, `leaves` and `pending` prints an array with fixed field names, and a
failure names itself on stderr and exits non-zero.

## Where it keeps things

```
~/.config/whypkg/config.toml    which sources to skip; o in the browser writes it
```

The file keeps your comments, so it can live in your dotfiles.

## Notes

- whypkg only reads: it never syncs, installs or removes anything, and the upgrade list is as fresh as your last `apt update`, `pacman -Sy`, `dnf makecache` or `zypper refresh`. A package's dossier shows the command that would remove it, for you to run.
- cargo, pipx, uv and npm are read from the files they keep (`~/.cargo/.crates2.json`, each tool's virtualenv, npm's global `node_modules`), never by running them, and their upgrades are not shown.
- A crate that starts other programs can name them in its `Cargo.toml` as `[package.metadata] runs = ["ssh", "rsync"]`, and whypkg links it to the packages that provide them once it is installed, because a program started at run time leaves no trace in the binary.
- Everything loads once into an in-memory graph, so every hop is a lookup rather than a subprocess; the slow part is your package manager's own queries, about half a second, and every other source is read while they run.
- It started as the bash `apt-why` and `apt-pending` scripts, kept in [`legacy/`](https://gitlab.com/safteinzz/whypkg/-/tree/main/legacy).

## Compatibility

Linux: Debian/Ubuntu (apt), Arch (pacman), Fedora/RHEL (dnf), openSUSE (zypper),
with flatpak, cargo, pipx, uv and npm alongside any of them, and the same browser
and the same reports on every one.

## License

AGPL-3.0-only
