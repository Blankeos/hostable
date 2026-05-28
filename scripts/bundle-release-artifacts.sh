#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
ADMIN_DIST="${ADMIN_DIST:-$ROOT/web/admin/dist}"
DIST_DIR="${DIST_DIR:-$ROOT/target/distrib}"

if [ ! -f "$ADMIN_DIST/index.html" ]; then
  echo "Missing admin web build at $ADMIN_DIST/index.html" >&2
  echo "Run: bun run build:admin" >&2
  exit 1
fi

found=0
for archive in "$DIST_DIR"/hostable-*unknown-linux-gnu.tar.xz; do
  [ -f "$archive" ] || continue
  found=1
  base="$(basename "$archive")"
  target="${base#hostable-}"
  target="${target%.tar.xz}"
  daemon_archive="$DIST_DIR/hostabled-$target.tar.xz"

  if [ ! -f "$daemon_archive" ]; then
    if tar -tf "$archive" | grep -q '/hostabled$'; then
      echo "Already bundled: $base"
      continue
    fi
    echo "Missing daemon archive: $daemon_archive" >&2
    exit 1
  fi

  work="$(mktemp -d)"
  mkdir -p "$work/tui" "$work/daemon" "$work/out/hostable-$target/web-admin"
  tar -xf "$archive" -C "$work/tui"
  tar -xf "$daemon_archive" -C "$work/daemon"
  cp "$work/tui/hostable-$target/hostable" "$work/out/hostable-$target/hostable"
  cp "$work/daemon/hostabled-$target/hostabled" "$work/out/hostable-$target/hostabled"
  cp -R "$ADMIN_DIST"/. "$work/out/hostable-$target/web-admin/"
  tar -C "$work/out" -cJf "$archive" "hostable-$target"

  if [ -f "$archive.sha256" ]; then
    (cd "$(dirname "$archive")" && sha256sum "$(basename "$archive")") > "$archive.sha256"
  fi
  if [ -f "$archive.sha256sum" ]; then
    (cd "$(dirname "$archive")" && sha256sum "$(basename "$archive")") > "$archive.sha256sum"
  fi

  rm -f "$daemon_archive" "$daemon_archive.sha256" "$daemon_archive.sha256sum"
  rm -rf "$work"
  echo "Bundled: $base"
done

if [ "$found" -eq 0 ]; then
  echo "No hostable release archives found in $DIST_DIR" >&2
  exit 1
fi

for archive in "$DIST_DIR"/hostable-*unknown-linux-gnu.tar.xz; do
  tar -tf "$archive" | grep '/hostable$' >/dev/null
  tar -tf "$archive" | grep '/hostabled$' >/dev/null
  tar -tf "$archive" | grep '/web-admin/index.html$' >/dev/null
done
