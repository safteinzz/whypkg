#!/usr/bin/env bash
# Open a shell inside the staged fake machine.
#
# bwrap gives the shell its own mount and environment view: the shims come first
# on PATH, and demo/home/var/log is bound over /var/log so whypkg's direct reads
# of dpkg.log and apt/history.log land in the fixtures too. Nothing outside this
# process sees either change, and nothing on the real system is written.
#
# /var/lib/flatpak is masked with an empty tmpfs for the same reason. whypkg
# reads an app's deploy directory mtime to date a flatpak install, and it looks
# at the system-wide path before the per-user one - so without the mask a real
# machine's real install dates end up in the pictures. That leak was in a frame
# before it was caught; check every path whypkg reads directly before adding one.
#
# The prompt is the same invented `user@host` every other crate's rig uses, and
# no tape sets a VHS theme, so every frame across the projects is the same
# terminal. It is not about hiding a name: these images are a build output, and
# one that comes out different on every machine that regenerates it is not
# reproducible.
set -euo pipefail

cd "$(dirname "$0")"
home=$PWD/home
[ -d "$home/bin" ] || { echo "enter.sh: run ./stage.sh first"; exit 1; }
command -v bwrap >/dev/null || { echo "enter.sh needs bubblewrap (bwrap)"; exit 1; }

# --clearenv is the bwrap spelling of `env -i`: what follows is the whole
# environment, not the real one plus overrides, so no variable nobody thought of
# survives to point whypkg at something real.
exec bwrap \
  --dev-bind / / \
  --bind "$home/var/log" /var/log \
  --tmpfs /var/lib/flatpak \
  --clearenv \
  --setenv PATH "$home/bin:/usr/bin:/bin" \
  --setenv HOME "$home" \
  --setenv PS1 '\[\e[38;5;114m\]user@host\[\e[0m\]:\[\e[38;5;110m\]\w\[\e[0m\]\$ ' \
  --setenv HISTFILE /dev/null \
  --setenv TERM "${TERM:-xterm-256color}" \
  --setenv COLORTERM truecolor \
  --setenv LANG C.UTF-8 \
  --chdir "$home" \
  bash --norc --noprofile "$@"
