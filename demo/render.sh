#!/usr/bin/env bash
# Render the README assets in a container, so a machine needs podman or docker
# and nothing else: no vhs, no bwrap, no font, and the same frames on every
# machine that runs it.
#
#   ./render.sh              every tape
#   ./render.sh demo         one tape
#
# Each tape runs in the image from ./Dockerfile, in a container of its own that
# stages, records and tears down. The image is Debian, so its own /var/log holds
# a real apt history: the container covers /var/log with an empty tmpfs that
# gets the stage's logs, and /var/lib/flatpak with an empty one, which is what
# enter.sh does with bwrap on a host and checks for here (`DEMO_MASKS`).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

ENGINE="$(command -v podman || command -v docker || true)"
[ -n "$ENGINE" ] || { echo "render.sh needs podman or docker" >&2; exit 1; }
IMAGE=localhost/whypkg-render

TAPES=("$@")
[ ${#TAPES[@]} -gt 0 ] || TAPES=(shots report demo)

(cd .. && cargo build --release)
# The Dockerfile is the whole build context: nothing in this folder is copied in.
"$ENGINE" build -q -t "$IMAGE" - < Dockerfile > /dev/null

# Run as this user, not root: the images it writes stay yours, and the staged
# shell's prompt ends in `$` as it does on a machine rendering without a
# container, rather than root's `#`.
if [ "$(basename "$ENGINE")" = docker ]; then
  USER_ARGS=(--user "$(id -u):$(id -g)" -e HOME=/tmp)
  LOG_MASK=/var/log:rw,mode=1777
else
  USER_ARGS=(--userns=keep-id -e HOME=/tmp)
  # podman copies the image's own /var/log into a tmpfs unless told not to.
  LOG_MASK=/var/log:rw,mode=1777,notmpcopyup
fi

# No network: the stage replays captured fixtures, so nothing in a take has a
# reason to leave the machine. The tapes run from the repo root.
for t in "${TAPES[@]}"; do
  echo "── $t.tape"
  "$ENGINE" run --rm --network none "${USER_ARGS[@]}" \
    --tmpfs "$LOG_MASK" --tmpfs /var/lib/flatpak \
    -v "$(cd .. && pwd):/work/whypkg:Z" -w /work/whypkg \
    -e DEMO_MASKS=container --entrypoint bash "$IMAGE" \
    -c "demo/stage.sh > /dev/null && cp -r demo/home/var/log/. /var/log/ && vhs demo/$t.tape > /dev/null; s=\$?; demo/stage.sh down > /dev/null; exit \$s"
done
