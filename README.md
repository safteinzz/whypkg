# apt-why

Understand every upgradable package on your Debian/Ubuntu system — grouped by
why it's there, not just listed.

Run it after `sudo apt update` and know exactly what you're about to upgrade
before you do it.

---

## The problem it solves

```
$ apt list --upgradable

libasan8         11 MB
lib32stdc++6     2976 KB
kaddressbook     5793 KB
exfatprogs       365 KB
...
```

That list tells you nothing. Why is `libasan8` here? Did you install
`kaddressbook` or did something else? Is it safe to upgrade `libstdc++6`?

`apt-why` answers all of that.

---

## Install

```bash
sudo cp apt-why /usr/local/bin/apt-why
sudo chmod +x /usr/local/bin/apt-why
```

---

## Usage

```
apt-why [options]
```

| Option | What it shows |
|---|---|
| *(none)* | Full report — everything below |
| `--quick` | Every package in one table: size + reason |
| `--kernel` | Kernel, firmware, microcode, foundational libs |
| `--apps` | Packages you explicitly installed |
| `--auto` | Auto-installed packages, grouped by what caused them |
| `--sizes` | Top 20 packages by installed size |
| `--all` | Explicit alias for the full report |
| `--help` | Usage and examples |

Options can be combined freely:

```bash
apt-why --auto --sizes
apt-why --kernel --apps
```

---

## Workflow

```bash
sudo apt update
apt-why --quick        # scan everything in ~2 seconds
apt-why --auto         # drill into a specific group if something looks off
sudo apt upgrade       # upgrade with confidence
```

---

## Output sections

### `--quick` — the daily driver

One line per package. Everything sorted by size so the biggest changes are
immediately obvious.

```
kmail                     25 MB   <- task-kde-desktop
intel-microcode           20 MB   kernel
libasan8                  11 MB   <- clang
libstdc++6              3152 KB   system lib
ckb-next                3427 KB   you
```

The tag on the right tells you everything:

| Tag | Meaning |
|---|---|
| `you` | You manually installed this |
| `kernel` | Kernel, firmware, or microcode |
| `system lib` | Foundational library — half the OS depends on it |
| `<- clang` | Auto-installed because you have `clang` |
| `auto` | Auto-installed, root package not in upgrade list |

### `--kernel` — kernel and system libs

Kernel meta-packages, firmware, microcode, and foundational runtime libraries
(`libgcc-s1`, `libstdc++6`, `libfreetype6` etc.). Foundational libs show how
many installed packages depend on them.

```
intel-microcode     20 MB   [you installed this]
Processor microcode firmware for Intel CPUs
3.20260210.1  ->  3.20260227.1

libstdc++6        3152 KB   754 packages depend on this
GNU Standard C++ Library v3
15.2.0-14  ->  16-20260308-1
```

### `--apps` — your installs

Packages marked as manually installed (excluding kernel and foundational libs).
Shows whether each one is standalone or has other packages depending on it.

```
>> ckb-next                  3427 KB
   driver for Corsair keyboards and mice
   standalone — nothing else on your system needs this

>> libunistring5              2102 KB
   Unicode string library for C
   6 other packages depend on this
```

A `standalone` package with no rdeps is a clean removal candidate if you no
longer need it.

### `--auto` — grouped by cause

Auto-installed packages grouped by the manual install that originally caused
them. Sorted by group size, biggest first.

```
PULLED IN BY: task-kde-desktop  (21 packages)
  All 21 packages below exist because you installed task-kde-desktop.

  kmail               25 MB    full featured graphical email client
  kdepim-addons       14 MB    Addons for KDE PIM applications
  kaddressbook      5793 KB    address book and contact data manager
  ...

PULLED IN BY: clang  (10 packages)
  All 10 packages below exist because you installed clang.

  libasan8            11 MB    AddressSanitizer — a fast memory error detector
  libtsan2          9184 KB    ThreadSanitizer — a Valgrind-based detector
  ...
```

This makes it immediately obvious: "I have 25MB of KDE email software upgrading
because of `task-kde-desktop`" or "I have 50MB of LLVM sanitizers because of
`clang`."

### `--sizes` — disk usage

Top 20 upgradable packages by installed size, with category tags. Useful for
spotting unexpectedly large upgrades before they happen.

---

## How it works

Speed was a key design goal — the original version took ~45 seconds. The
current version runs in 2-3 seconds by doing all data collection upfront in
three bulk calls:

1. **`apt list --upgradable`** — gets the package list and version info in one
   shot. Multi-arch duplicates (same package listed for `amd64` and `i386`) are
   collapsed automatically.

2. **`apt-cache show pkg1 pkg2 ...`** — fetches size and description for all
   upgradable packages in a single call, parsed with one `awk` pass into
   in-memory arrays.

3. **`dpkg-query -W -f='${Package}\t${Depends}\t${Recommends}'`** — dumps the
   entire installed dependency graph (both `Depends` and `Recommends`) in one
   call. This is inverted into a reverse-dependency map, so BFS traversal
   during classification is pure bash array lookups — zero subprocess forks.

Classification priority (matches the dep graph traversal order):

```
kernel  >  foundational lib (>25 rdeps)  >  auto-with-traced-root  >  manual app  >  untraced
```

The foundational lib threshold (25 rdeps) prevents packages like `libgcc-s1`
(313 rdeps) or `libstdc++6` (754 rdeps) from generating useless 200-line
dependency trees.

---

## Spotting ghost manual marks

If a package shows as `you` but you don't remember installing it, it probably
has a stale manual mark — common on long-lived systems after removing packages
that had deps.

```bash
# Check what actually depends on it
apt-cache rdepends --installed <package>

# Check install history (may have rotated)
grep <package> /var/log/apt/history.log

# If you're happy it's just a ghost:
sudo apt-mark auto <package>
```

Marking it auto won't remove it if anything depends on it. It just means apt
will clean it up automatically when the last thing needing it is removed.

---

## Requirements

- Bash 4.0+
- `apt`, `apt-cache`, `apt-mark`, `dpkg-query` (standard on any Debian/Ubuntu system)
- No external dependencies
