#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
PORT="${HOSTABLE_MINECRAFT_SMOKE_PORT:-18088}"
BASE="http://127.0.0.1:$PORT"
TOKEN="minecraft-smoke-token"
WORK="$(mktemp -d)"
SERVER_DIR="$WORK/server"
VERSION="${HOSTABLE_MINECRAFT_VERSION:-1.20.4}"
TYPE="${HOSTABLE_MINECRAFT_TYPE:-vanilla}"
MEMORY_MB="${HOSTABLE_MINECRAFT_MEMORY_MB:-768}"
TIMEOUT_SECONDS="${HOSTABLE_MINECRAFT_BOOT_TIMEOUT:-120}"
HOSTABLED_BIN="${HOSTABLED_BIN:-$ROOT/target/debug/hostabled}"

cleanup() {
  if [ "${SERVER_PID:-}" ]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  if [ "${DAEMON_PID:-}" ]; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

java_major() {
  java -version 2>&1 | awk -F '"' '/version/ {
    split($2, parts, ".")
    if (parts[1] == "1") print parts[2]; else print parts[1]
  }'
}

if ! command -v java >/dev/null 2>&1; then
  echo "Skipping live Minecraft smoke: java is not installed." >&2
  exit 0
fi

JAVA_MAJOR="$(java_major)"
if [ "${JAVA_MAJOR:-0}" -lt 17 ]; then
  echo "Skipping live Minecraft smoke: Java 17+ is required, found Java $JAVA_MAJOR." >&2
  exit 0
fi

mkdir -p "$WORK/state" "$WORK/config" "$WORK/backups" "$SERVER_DIR"
printf '%s' "$TOKEN" > "$WORK/config/operator-token"
printf '{"admin_web":{"enabled":false,"domain":null}}' > "$WORK/config/settings.json"

if [ ! -x "$HOSTABLED_BIN" ] && [ "$HOSTABLED_BIN" = "$ROOT/target/debug/hostabled" ]; then
  (cd "$ROOT" && cargo build -p hostabled)
fi
if [ ! -x "$HOSTABLED_BIN" ]; then
  echo "hostabled binary is not executable: $HOSTABLED_BIN" >&2
  exit 1
fi

"$HOSTABLED_BIN" \
  --bind "127.0.0.1:$PORT" \
  --registry "$WORK/state/registry.sqlite" \
  --backup-root "$WORK/backups" \
  --settings "$WORK/config/settings.json" \
  --admin-dist "$ROOT/web/admin/dist" \
  --admin-password-hash "$WORK/config/admin-password.hash" \
  --operator-token "$WORK/config/operator-token" &
DAEMON_PID="$!"

tries=0
until curl -fsS "$BASE/api/health" > "$WORK/health.json" 2>/dev/null; do
  tries=$((tries + 1))
  if [ "$tries" -gt 50 ]; then
    echo "hostabled did not become ready on $BASE" >&2
    exit 1
  fi
  sleep 0.1
done

curl -fsS -X POST "$BASE/api/servers/create" \
  -H "x-hostable-operator-token: $TOKEN" \
  -H 'content-type: application/json' \
  -d "{\"game\":\"minecraft\",\"dry_run\":true,\"input\":{\"name\":\"Live Minecraft Smoke\",\"install_path\":\"$SERVER_DIR\",\"server_type\":\"$TYPE\",\"game_version\":\"$VERSION\",\"loader_version\":null,\"build_version\":null,\"memory_mb\":$MEMORY_MB,\"accepted_license\":true,\"extra\":{\"motd\":\"hostable smoke\",\"rcon_password\":\"smoke-rcon\"}}}" \
  > "$WORK/plan.json"

python3 - "$WORK/plan.json" "$SERVER_DIR" <<'PY'
import json
import os
import shutil
import subprocess
import sys

plan_path, server_dir = sys.argv[1], sys.argv[2]
with open(plan_path, "r", encoding="utf-8") as handle:
    plan = json.load(handle)

for action in plan["plan"]["actions"]:
    if "CreateDirectory" in action:
        path = action["CreateDirectory"]["path"]
        if path.startswith(server_dir):
            os.makedirs(path, exist_ok=True)
    elif "WriteFile" in action:
        item = action["WriteFile"]
        path = item["path"]
        if not path.startswith(server_dir):
            continue
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(item["contents"])
        os.chmod(path, item["mode"])
    elif "RunCommand" in action:
        item = action["RunCommand"]
        program = item["program"]
        args = item["args"]
        cwd = item.get("cwd")
        if cwd and not cwd.startswith(server_dir):
            continue
        if program not in {"curl", "python3"}:
            continue
        if shutil.which(program) is None:
            raise SystemExit(f"required command not found: {program}")
        subprocess.run([program, *args], cwd=cwd or server_dir, check=True)
PY

if [ ! -s "$SERVER_DIR/server.jar" ]; then
  echo "Minecraft smoke did not produce server.jar" >&2
  exit 1
fi
test -s "$SERVER_DIR/server.properties"
test -s "$SERVER_DIR/eula.txt"
grep -q 'eula=true' "$SERVER_DIR/eula.txt"

mkfifo "$WORK/server.in"
(cd "$SERVER_DIR" && java -Xms"${MEMORY_MB}"M -Xmx"${MEMORY_MB}"M -jar server.jar nogui < "$WORK/server.in" > "$WORK/server.log" 2>&1) &
SERVER_PID="$!"
exec 3> "$WORK/server.in"

elapsed=0
until grep -q 'Done (' "$WORK/server.log"; do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    cat "$WORK/server.log" >&2
    echo "Minecraft server exited before becoming ready." >&2
    exit 1
  fi
  if [ "$elapsed" -ge "$TIMEOUT_SECONDS" ]; then
    cat "$WORK/server.log" >&2
    echo "Minecraft server did not become ready within ${TIMEOUT_SECONDS}s." >&2
    exit 1
  fi
  elapsed=$((elapsed + 1))
  sleep 1
done

printf 'stop\n' >&3
exec 3>&-
wait "$SERVER_PID"
SERVER_PID=""

grep -q 'Stopping server' "$WORK/server.log"
echo "minecraft-live-smoke-ok"
