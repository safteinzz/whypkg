# whypkg

> **Canonical:** [gitlab.com/safteinzz/whypkg](https://gitlab.com/safteinzz/whypkg) · **Mirror:** [github.com/safteinzz/whypkg](https://github.com/safteinzz/whypkg)

<!-- desc:start -->
why is that package here? know it now - a fast, cross-distro package investigator (apt, pacman, dnf, flatpak)
<!-- desc:end -->

## Install

```bash
cargo install whypkg
whypkg self check   # is a newer release out?
whypkg self update  # install the latest
```

No cargo yet? Rust installs the same way on every distro: [rustup.rs](https://rustup.rs).

## What have I actually got?

![The whypkg browser listing every installed package, each row tagged with how it got there](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/browse.png)

Legend: `[M]` you installed it · `[A]` something pulled it in · `[F]` flatpak app · `↑` upgrade waiting

```bash
whypkg                 # browse every installed package
```

## What was that thing called?

![Typing "element" as a filter, with the flatpak app im.riot.Riot at the top of the results](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/search.png)

`/` matches descriptions as well as names, which is the only way anyone finds a
flatpak app by the name on its window.

## Why is this here?

![A package dossier: libllvm21, 133 MB, pulled in by clang, with the eleven packages that need it listed below](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/dossier.png)

**pulled in by clang.** That is the whole point. You also get where it came
from, what needs it and what it needs, and `↵` on anything in the list follows
the thread.

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
whypkg pending --kernel   # one section at a time; --apps, --auto and --sizes too
```

`whypkg <command> --help` has the details, and `?` lists every key.

`pending --quick` prints one line per package for a pipe, everything else is
rendered for people, and a failure names itself on stderr and exits non-zero.

## Notes

- whypkg only reads: it never syncs, installs or removes anything, and the upgrade list is as fresh as your last `apt update`, `pacman -Sy` or `dnf makecache`.
- Everything loads once into an in-memory graph, so every hop is a lookup rather than a subprocess; the slow part is your package manager's own queries, about half a second.
- It started as the bash `apt-why` and `apt-pending` scripts, kept in [`legacy/`](https://gitlab.com/safteinzz/whypkg/-/tree/main/legacy).

## Compatibility

Linux: Debian/Ubuntu (apt), Arch (pacman), Fedora/RHEL (dnf), and flatpak apps
alongside any of them, with the same browser and the same report on every one.

## License

AGPL-3.0-only
