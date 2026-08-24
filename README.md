# whypkg

> **Canonical:** [gitlab.com/safteinzz/whypkg](https://gitlab.com/safteinzz/whypkg) · **Mirror:** [github.com/safteinzz/whypkg](https://github.com/safteinzz/whypkg)

<!-- desc:start -->
wonder why the f* you have that package? know it now - a fast, cross-distro package investigator (apt, pacman, dnf, flatpak)
<!-- desc:end -->

## Install

```bash
cargo install whypkg
whypkg self check   # is a newer release out?
whypkg self update  # install the latest
```

No cargo yet? Rust installs the same way on every distro: [rustup.rs](https://rustup.rs).

![whypkg filtering a package list, opening a package to see what pulled it in, following the dependency graph, and printing the pending-upgrade report](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/demo.gif)

## What have I actually got?

`[M]` you installed it · `[A]` something pulled it in · `[F]` flatpak app · `↑` upgrade waiting

![The whypkg browser listing every installed package, each row tagged with how it got there](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/browse.png)

## What was that thing called?

Type to filter. It matches descriptions as well as names, which is the only way
anyone finds a flatpak app by the name on its window.

![Typing "element" as a filter, with the flatpak app im.riot.Riot at the top of the results](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/search.png)

## Why is this here?

![A package dossier: libllvm21, 133 MB, pulled in by clang, with the eleven packages that need it listed below](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/dossier.png)

**pulled in by clang.** That is the whole point. You also get what it arrived
alongside, whether it came from a repo or a local file you sideloaded (or a repo
that no longer exists), what needs it and what it needs. `Enter` on anything in
the list follows the thread; `Esc` comes back.

## What is around it?

`Ctrl-G` draws the neighbourhood in the terminal. No browser, no image protocol,
so it survives ssh and tmux.

![The dependency graph view: libllvm21 in the centre, packages that need it on the left, packages it needs on the right](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/graph.png)

`Enter` re-centres on a neighbour and keeps digging, `Esc` retraces.

## What is this upgrade about to pull down?

![The pending report: one line per upgradable package with its size and why it is installed](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/pending.png)

Every package with an upgrade waiting, and why it is on your machine. Pipe it,
grep it, diff it before and after.

## Commands

```bash
whypkg                 # browse every installed package
whypkg --upgradable    # browse only packages with an upgrade waiting
whypkg pending         # full report, grouped by what pulled things in
whypkg pending --quick # one line per package: size + reason
```

`whypkg pending` also takes `--kernel`, `--apps`, `--auto` and `--sizes` to show
one section at a time.

## It never touches your system

whypkg only reads. It will not sync, install or remove anything. The upgrade
list reflects your last database refresh (`apt update`, `pacman -Sy`,
`dnf makecache`), exactly like your package manager's own listing does.

Everything loads once at startup into an in-memory graph, so every hop while you
browse is a hash-map lookup rather than a subprocess. The slow part is your
package manager's own queries, about half a second.

It started as the bash `apt-why` / `apt-pending` scripts, kept in
[`legacy/`](https://gitlab.com/safteinzz/whypkg/-/tree/main/legacy).

## Compatibility

Linux: Debian/Ubuntu (apt), Arch (pacman), Fedora/RHEL (dnf), and flatpak apps
alongside any of them. Each package manager sits behind one `Backend` trait, so
the analysis and the interface are distro-agnostic.

## License

AGPL-3.0-only
