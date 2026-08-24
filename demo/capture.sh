#!/usr/bin/env bash
# Regenerate the captured Debian package data in fixtures/.
#
# Rare: only when the demo world should hold different packages. Needs podman
# and network; everything else in demo/ runs offline from what this writes.
#
# It installs the demo's package set into a throwaway debian:sid container in
# *stages*, snapshotting the installed set after each one. The diff between two
# snapshots is exactly what that stage pulled in, which is what lets stage.sh
# write an install log where `libaom3` really was installed the day `ffmpeg`
# was, rather than a timeline someone guessed.
#
# Nothing here touches the host: the container is --rm and the only shared path
# is fixtures/, mounted read-write.
set -euo pipefail

cd "$(dirname "$0")"
out="$PWD/fixtures"
mkdir -p "$out"

command -v podman >/dev/null || { echo "capture.sh needs podman"; exit 1; }

# Each line is one install session: a label, how the packages should end up
# marked, then the packages asked for. stage.sh maps the labels, in this order,
# onto dates.
#
# The `auto` session is the dependency set of the sideloaded .deb that fixtures/
# adds by hand. apt-get would mark anything named on its command line as
# manually installed, which would have whypkg answer "you installed this" about
# a library nobody chose; marking them auto afterwards is what lets the trace
# come out as "pulled in by code".
sessions=(
  "tools:manual:git curl htop tmux zsh"
  "clang:manual:clang"
  "media:manual:ffmpeg imagemagick"
  "editor:manual:neovim ripgrep fzf bat jq"
  "sideload:auto:libnss3 libgtk-3-0t64 libasound2t64 libxkbfile1 libsecret-1-0 xdg-utils"
  "recent:manual:nodejs sqlite3"
)

script='
set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
snap() { dpkg-query -W -f="\${Package}\n" | sort > "/cap/.snap.$1"; }
snap base
'
for entry in "${sessions[@]}"; do
  IFS=: read -r label mode pkgs <<<"$entry"
  script+="apt-get install -y -qq $pkgs >/dev/null
"
  [ "$mode" = auto ] && script+="apt-mark auto $pkgs >/dev/null
"
  script+="snap $label
"
done

# Final captures, in the exact shapes src/backend/apt.rs asks for.
script+='
export LC_ALL=C
dpkg-query -W -f="\${Package}\t\${Version}\t\${Installed-Size}\t\${Description}\n" > /cap/apt-meta.tsv
dpkg-query -W -f="\${Package}\t\${Depends}\t\${Recommends}\n"                     > /cap/apt-deps.tsv
apt-mark showmanual                                                            > /cap/apt-manual.txt
apt list --installed 2>/dev/null                                               > /cap/apt-installed.txt

# Turn the snapshots into "session<TAB>package", earliest session wins.
prev=/cap/.snap.base
: > /cap/apt-sessions.tsv
while read -r label; do
  comm -13 "$prev" "/cap/.snap.$label" | sed "s/^/$label\t/" >> /cap/apt-sessions.tsv
  prev=/cap/.snap.$label
done < /cap/.labels
sed "s/^/base\t/" /cap/.snap.base >> /cap/apt-sessions.tsv
rm -f /cap/.snap.* /cap/.labels
'

printf '%s\n' "${sessions[@]%%:*}" > "$out/.labels"
podman run --rm -v "$out:/cap:z" docker.io/library/debian:sid bash -c "$script"

echo "captured $(wc -l < "$out/apt-meta.tsv") metadata lines into fixtures/"
