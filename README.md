# whypkg

> **Canonical:** [gitlab.com/safteinzz/whypkg](https://gitlab.com/safteinzz/whypkg) · **Mirror:** [github.com/safteinzz/whypkg](https://github.com/safteinzz/whypkg)

**Wonder why the f\* you have that package? Know it now. 🎯**

A fast, cross-distro package investigator for apt, pacman, dnf and flatpak.
Fuzzy-find anything installed and find out: did *you* install it, or did
something drag it in?

```bash
cargo install whypkg
```

## Browse everything

`[M]` you installed it · `[A]` something pulled it in · `[F]` flatpak app · `↑` upgrade waiting

![Browsing installed packages](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/browse.png)

Type to fuzzy-filter. It searches descriptions too, so `visual studio` finds
`code` and `element` finds `im.riot.Riot`. `Tab` cycles all / manual / auto /
flatpak.

## Open a package and get the answer

![A package dossier](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/dossier.png)

**pulled in by clang.** That is the whole point of the tool. You also get what
it came installed alongside, whether it came from a repo or a local file you
sideloaded (or a repo that no longer exists), what needs it, and what it needs.
Press `Enter` on anything in the list to follow the thread, `Esc` to come back.

## See the neighbourhood

Press `Ctrl-G` on any package for a live dependency graph, drawn in the
terminal. No browser, no image protocol, works over ssh and inside tmux.

![The dependency graph](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/graph.png)

`hjkl` or the arrows to move between planets, `Enter` to re-centre on one and
keep digging, `Esc` to retrace your steps.

## Know what an upgrade is about to pull down

```bash
whypkg pending --quick
```

![The pending upgrade report](https://gitlab.com/safteinzz/whypkg/-/raw/main/readme-assets/pending.png)

Every package with an upgrade waiting, and why it is on your system. Pipe it,
grep it, diff it before and after.

## Commands

```bash
whypkg                 # browse every installed package
whypkg --upgradable    # browse only packages with an upgrade waiting
whypkg pending         # full report, grouped by what pulled things in
whypkg pending --quick # one line per package: size + reason
whypkg update          # update whypkg itself
```

`whypkg pending` also takes `--kernel`, `--apps`, `--auto` and `--sizes` to show
one section at a time.

## Distro support

| Distro family         | Status     |
|-----------------------|------------|
| Debian / Ubuntu (apt) | ✅ working  |
| Arch (pacman)         | ✅ working  |
| Fedora / RHEL (dnf)   | ✅ working  |
| Flatpak               | ✅ working  |

Flatpak apps show up alongside your system packages when flatpak is installed.
Each package manager lives behind a single `Backend` trait, so the analysis and
the UI are distro-agnostic.

## It never touches your system

whypkg only reads. It will not sync, install or remove anything. Upgrade lists
reflect your last database refresh (`apt update`, `pacman -Sy`, `dnf makecache`),
the same way your package manager's own "what can be upgraded" listing does.

## Why it is fast

Everything loads once at startup into an in-memory graph, so every hop while you
browse is a hash-map lookup rather than a subprocess. The slow part is your
package manager's own queries (about half a second), not whypkg.

It started life as the bash `apt-why` / `apt-pending` scripts, kept in
[`legacy/`](legacy/).

## License

AGPL-3.0-only.
