#!/usr/bin/env bash
# Build the fake machine the demo is filmed against, in demo/home/.
#
# whypkg reads a system two ways: it shells out to the package manager, and it
# reads /var/log directly. So the stage is two halves:
#
#   * demo/home/bin/    shim `apt`, `apt-mark`, `dpkg-query` and `flatpak` that
#                       replay fixtures/ instead of asking this machine
#   * demo/home/var/log a dpkg log and an apt history log written from the
#                       session data in fixtures/apt-sessions.tsv
#
# enter.sh then puts the first on PATH and bind-mounts the second over /var/log,
# so whypkg sees an invented Debian workstation and never reads a real one. That
# also means the rig renders the same pictures on a machine with no apt at all.
#
# Install dates are written relative to today, so the dossier always reads
# "2 months ago" rather than drifting to "3 years ago" as the fixtures age.
#
# Everything it writes lives under demo/home/, which it deletes first and which
# is gitignored. It never writes anywhere else.
set -euo pipefail

cd "$(dirname "$0")"
demo=$PWD
fixtures=$demo/fixtures
STAGE=$demo/home
home=$STAGE

# Written by this script at the end of a build, required before it deletes the
# previous one. See the guard below.
MARKER=".whypkg-demo-stage"

# ---------------------------------------------------------------------------
# the teardown guard - identical in every crate's rig
# ---------------------------------------------------------------------------
# A rig is a convenience script with a recursive delete in it, run half
# attentively while thinking about something else, against a path some scenario
# may have mounted a remote filesystem onto. Both halves of that have already
# happened in this workflow: a stage path that pointed somewhere real and was
# deleted because the script trusted its own variable, and an sshfs mount inside
# a staged home torn down with `rm -rf`, which walked through the mountpoint and
# deleted the dotfiles on the machine at the far end. So the delete is proved
# rather than trusted.
refuse() { echo "REFUSING to delete $STAGE: $1" >&2; exit 1; }

assert_safe_to_delete() {
  case "$STAGE" in
    /*) ;;
    *) refuse "the stage path must be absolute" ;;
  esac
  # Resolve symlinks first: a link pointing the stage at something real must not
  # let a delete through on the strength of a harmless-looking path.
  local real
  real="$(cd "$STAGE" && pwd -P)" || refuse "cannot resolve the path"
  case "$real" in
    / | /home | /root | /usr | /etc | /var | /opt | /srv | /boot | /tmp)
      refuse "that is a system directory" ;;
  esac
  [ "$real" = "$HOME" ] && refuse "that is your home directory"
  case "$HOME/" in
    "$real"/*) refuse "your home directory is inside it" ;;
  esac
  # The real gate: only ever delete a tree this script built and stamped.
  [ -f "$real/$MARKER" ] || refuse "no \`$MARKER\` in it, so this script did not build it"
  # Unmount anything under it, longest path first, then check again: a recursive
  # delete walks straight through a mountpoint and removes the far side.
  local mp
  while read -r mp; do
    [ -n "$mp" ] || continue
    echo "unmounting $mp"
    fusermount -u "$mp" 2> /dev/null || umount "$mp" 2> /dev/null || true
  done < <(awk -v s="$real/" '$2 ~ "^"s {print length($2), $2}' /proc/mounts |
             sort -rn | cut -d' ' -f2-)
  if awk -v s="$real/" '$2 ~ "^"s {found=1} END {exit !found}' /proc/mounts; then
    refuse "something is still mounted under it; unmount it by hand and rerun"
  fi
}

# Rebuilding starts by removing the last stage, through the guard above.
# --one-file-system as a second net, in case the mount check was wrong.
if [ -d "$STAGE" ]; then
  assert_safe_to_delete
  rm -rf --one-file-system "$STAGE"
fi

binary=$demo/../target/release/whypkg
[ -x "$binary" ] || { echo "stage.sh: build first - cargo build --release"; exit 1; }
[ -s "$fixtures/apt-meta.tsv" ] || { echo "stage.sh: fixtures missing - run ./capture.sh"; exit 1; }

mkdir -p "$home/bin" "$home/var/log/apt" "$home/.local/share/flatpak/app"
# Stamp it, so the next run can prove this tree is the one this script built.
: > "$STAGE/$MARKER"

# ── the world, assembled from captured + invented packages ──────────────────
world=$home/world
mkdir -p "$world"
cat "$fixtures/apt-meta.tsv"   "$fixtures/extra-meta.tsv"   > "$world/meta.tsv"
cat "$fixtures/apt-deps.tsv"   "$fixtures/extra-deps.tsv"   > "$world/deps.tsv"
cat "$fixtures/apt-manual.txt" "$fixtures/extra-manual.txt" | sort -u > "$world/manual.txt"
cp "$fixtures/flatpak-apps.tsv"     "$world/flatpak-apps.tsv"
cp "$fixtures/flatpak-runtimes.tsv" "$world/flatpak-runtimes.tsv"

# `apt list --installed`: the capture, plus the invented packages. The sideload
# and the orphan carry the `[installed,local]` tag - "no configured repo offers
# this" - which is what makes whypkg ask the history log about them.
{
  cat "$fixtures/apt-installed.txt"
  while IFS=$'\t' read -r name version _ _; do
    [ -n "$name" ] || continue
    if grep -qxF "$name" "$fixtures/sideloads.txt" "$fixtures/orphans.txt" 2>/dev/null; then
      echo "$name/now $version amd64 [installed,local]"
    else
      echo "$name/testing,now $version amd64 [installed]"
    fi
  done < "$fixtures/extra-meta.tsv"
} > "$world/installed.txt"

# `apt list --upgradable`: fixtures/upgrades.tsv is name + the version waiting.
: > "$world/upgradable.txt"
echo "Listing..." >> "$world/upgradable.txt"
while IFS=$'\t' read -r name candidate; do
  [ -n "$name" ] || continue
  current=$(awk -F'\t' -v n="$name" '$1==n {print $2; exit}' "$world/meta.tsv")
  [ -n "$current" ] || continue
  echo "$name/testing $candidate amd64 [upgradable from: $current]" >> "$world/upgradable.txt"
done < "$fixtures/upgrades.tsv"

# ── the shims ───────────────────────────────────────────────────────────────
write_shim() { printf '#!/bin/sh\n%s\n' "$2" > "$home/bin/$1"; chmod +x "$home/bin/$1"; }

write_shim dpkg-query '
case "$*" in
  *Depends*) exec cat "$(dirname "$0")/../world/deps.tsv" ;;
  *)         exec cat "$(dirname "$0")/../world/meta.tsv" ;;
esac'

write_shim apt-mark '
case "$1" in
  showmanual) exec cat "$(dirname "$0")/../world/manual.txt" ;;
esac'

write_shim apt '
w=$(dirname "$0")/../world
case "$*" in
  *--upgradable*) exec cat "$w/upgradable.txt" ;;
  *--installed*)  exec cat "$w/installed.txt" ;;
esac'

write_shim flatpak '
w=$(dirname "$0")/../world
case "$*" in
  *--runtime*) exec cat "$w/flatpak-runtimes.tsv" ;;
  *--app*)     exec cat "$w/flatpak-apps.tsv" ;;
esac'

ln -sf "$binary" "$home/bin/whypkg"

# ── the install history ─────────────────────────────────────────────────────
# One session per label, each on its own day, packages seconds apart within it.
# The order matters: whypkg reads this log to answer "installed alongside what?".
sessions="base:214 tools:96 clang:61 media:38 editor:17 sideload:9 recent:2"

log=$home/var/log/dpkg.log
: > "$log"
for entry in $sessions; do
  label=${entry%%:*}
  days=${entry##*:}
  day=$(date -u -d "$days days ago" +%Y-%m-%d)
  second=0
  awk -F'\t' -v l="$label" '$1==l {print $2}' "$fixtures/apt-sessions.tsv" |
  while read -r pkg; do
    version=$(awk -F'\t' -v n="$pkg" '$1==n {print $2; exit}' "$world/meta.tsv")
    stamp=$(date -u -d "$day 09:14:02 UTC + $second seconds" +%H:%M:%S)
    echo "$day $stamp install $pkg:amd64 <none> ${version:-1.0}"
    second=$((second + 7))
  done >> "$log"
done

# The sideload gets its own line in dpkg.log and a matching apt history entry
# whose Commandline names a .deb - that pairing is what whypkg reads as
# "you installed this from a local file" rather than "its repo went away".
history=$home/var/log/apt/history.log
: > "$history"
while read -r pkg; do
  [ -n "$pkg" ] || continue
  version=$(awk -F'\t' -v n="$pkg" '$1==n {print $2; exit}' "$world/meta.tsv")
  day=$(date -u -d '9 days ago' +%Y-%m-%d)
  echo "$day 20:31:07 install $pkg:amd64 <none> $version" >> "$log"
  cat >> "$history" <<EOF
Start-Date: $day  20:31:07
Commandline: apt install ./${pkg}_${version}_amd64.deb
Requested-By: demo (1000)
Install: $pkg:amd64 ($version)
End-Date: $day  20:31:44

EOF
done < "$fixtures/sideloads.txt"

sort -o "$log" "$log"

# ── flatpak install dates ───────────────────────────────────────────────────
# whypkg has no column for these; it uses the deploy directory's mtime.
offset=44
cut -f1 "$fixtures/flatpak-apps.tsv" | while read -r app; do
  [ -n "$app" ] || continue
  mkdir -p "$home/.local/share/flatpak/app/$app"
  touch -d "$(date -u -d "$offset days ago" +%Y-%m-%dT18:05:00)" \
        "$home/.local/share/flatpak/app/$app"
  offset=$((offset - 6))
done

echo "staged $(wc -l < "$world/meta.tsv") packages and $(wc -l < "$log") install events in demo/home/"
