#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
IMAGE="${HOSTABLE_MINECRAFT_SMOKE_IMAGE:-ubuntu:24.04}"
LINUX_HOSTABLED_BIN="$ROOT/target/x86_64-unknown-linux-gnu/dist/hostabled"

if ! command -v docker >/dev/null 2>&1; then
  echo "Docker is required for this smoke test." >&2
  exit 1
fi

if [ ! -x "$LINUX_HOSTABLED_BIN" ]; then
  echo "Missing Linux hostabled binary at $LINUX_HOSTABLED_BIN" >&2
  echo "Run: dist build --artifacts=local --target=x86_64-unknown-linux-gnu" >&2
  exit 1
fi

docker run --rm --platform linux/amd64 \
  -e HOSTABLE_MINECRAFT_BOOT_TIMEOUT \
  -e HOSTABLE_MINECRAFT_MEMORY_MB \
  -v "$ROOT:/work" \
  -w /work \
  "$IMAGE" \
  /bin/sh -lc '
    set -eu
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install -y curl ca-certificates python3 openjdk-17-jre-headless

    run_type() {
      type="$1"
      HOSTABLED_BIN=/work/target/x86_64-unknown-linux-gnu/dist/hostabled \
      HOSTABLE_MINECRAFT_MEMORY_MB="${HOSTABLE_MINECRAFT_MEMORY_MB:-1024}" \
      HOSTABLE_MINECRAFT_BOOT_TIMEOUT="${HOSTABLE_MINECRAFT_BOOT_TIMEOUT:-180}" \
      HOSTABLE_MINECRAFT_TYPE="$type" \
        scripts/smoke-minecraft-live.sh
    }

    run_type vanilla
    run_type paper
    run_type fabric
  '
