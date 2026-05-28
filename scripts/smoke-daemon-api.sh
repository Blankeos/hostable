#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
PORT="${HOSTABLE_SMOKE_PORT:-18087}"
BASE="http://127.0.0.1:$PORT"
TOKEN="smoke-token"
PASSWORD="long-enough-password"
WORK="$(mktemp -d)"

cleanup() {
  if [ "${DAEMON_PID:-}" ]; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

if [ ! -f "$ROOT/web/admin/dist/index.html" ]; then
  (cd "$ROOT" && bun run build:admin)
fi

(cd "$ROOT" && cargo build -p hostabled)

mkdir -p "$WORK/state" "$WORK/config" "$WORK/backups" "$WORK/server/world" "$WORK/bin"
printf '%s' "$TOKEN" > "$WORK/config/operator-token"
printf '{"admin_web":{"enabled":false,"domain":null}}' > "$WORK/config/settings.json"
printf 'world' > "$WORK/server/world/level.dat"
printf 'motd=old\n' > "$WORK/server/server.properties"
printf 'eula=true\n' > "$WORK/server/eula.txt"
cat > "$WORK/bin/systemctl" <<'SYSTEMCTL'
#!/usr/bin/env sh
set -eu
verb="${1:-}"
unit="${2:-}"
printf '%s %s\n' "$verb" "$unit" >> "$HOSTABLE_FAKE_SYSTEMCTL_LOG"
case "$verb" in
  is-active)
    if [ -f "$HOSTABLE_FAKE_SYSTEMCTL_STATE" ]; then
      cat "$HOSTABLE_FAKE_SYSTEMCTL_STATE"
    else
      printf 'inactive\n'
    fi
    ;;
  start|restart)
    printf 'active\n' > "$HOSTABLE_FAKE_SYSTEMCTL_STATE"
    ;;
  stop)
    printf 'inactive\n' > "$HOSTABLE_FAKE_SYSTEMCTL_STATE"
    ;;
esac
SYSTEMCTL
chmod +x "$WORK/bin/systemctl"
cat > "$WORK/bin/journalctl" <<'JOURNALCTL'
#!/usr/bin/env sh
set -eu
unit=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -u)
      unit="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
printf '2026-05-28T10:00:00+00:00 %s Hostable smoke log line\n' "$unit"
JOURNALCTL
chmod +x "$WORK/bin/journalctl"
export PATH="$WORK/bin:$PATH"
export HOSTABLE_FAKE_SYSTEMCTL_LOG="$WORK/systemctl.log"
export HOSTABLE_FAKE_SYSTEMCTL_STATE="$WORK/systemctl.state"

"$ROOT/target/debug/hostabled" \
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

code="$(curl -sS -o "$WORK/admin-disabled.html" -w '%{http_code}' "$BASE/")"
test "$code" = 404

curl -fsS -X PUT "$BASE/api/settings/admin-web" \
  -H "x-hostable-operator-token: $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"enabled":true,"domain":"play.example.com"}' \
  > "$WORK/settings.json"

curl -fsS "$BASE/" > "$WORK/admin-index.html"
grep -q '<div id="root"></div>' "$WORK/admin-index.html"

curl -fsS -c "$WORK/cookies.txt" -X POST "$BASE/api/admin/setup" \
  -H 'content-type: application/json' \
  -d "{\"password\":\"$PASSWORD\"}" \
  > "$WORK/setup.json"

curl -fsS -b "$WORK/cookies.txt" "$BASE/api/servers" > "$WORK/servers.json"

curl -fsS -X POST "$BASE/api/servers/create" \
  -H "x-hostable-operator-token: $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"game":"minecraft","dry_run":true,"input":{"name":"Fabric Friends","install_path":"/srv/hostable/servers/fabric-friends","server_type":"fabric","game_version":"1.20.4","loader_version":null,"build_version":null,"memory_mb":4096,"accepted_license":true,"extra":{"motd":"friends"}}}' \
  > "$WORK/minecraft-plan.json"
grep -q 'meta.fabricmc.net' "$WORK/minecraft-plan.json"

curl -fsS -X POST "$BASE/api/servers/create" \
  -H "x-hostable-operator-token: $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"game":"ark_survival_evolved","dry_run":true,"input":{"name":"Ark Friends","install_path":"/srv/hostable/servers/ark-friends","server_type":"ark_survival_evolved","game_version":null,"loader_version":null,"build_version":null,"memory_mb":null,"accepted_license":true,"extra":{"session_name":"Ark Friends"}}}' \
  > "$WORK/ark-plan.json"
grep -q 'steamcmd' "$WORK/ark-plan.json"

curl -fsS -X POST "$BASE/api/servers" \
  -H "x-hostable-operator-token: $TOKEN" \
  -H 'content-type: application/json' \
  -d "{\"id\":\"smoke-mc\",\"name\":\"Smoke MC\",\"game\":\"minecraft\",\"install_path\":\"$WORK/server\",\"service_name\":\"hostable-smoke-mc.service\",\"ports\":[{\"port\":25565,\"protocol\":\"tcp\",\"label\":\"Minecraft server\"}],\"adapter_config\":{\"extra\":{}}}" \
  > "$WORK/register.json"

curl -fsS "$BASE/api/servers/smoke-mc/status" \
  -H "x-hostable-operator-token: $TOKEN" \
  > "$WORK/status-inactive.json"
grep -q '"active":"inactive"' "$WORK/status-inactive.json"

curl -fsS -X POST "$BASE/api/servers/smoke-mc/start" \
  -H "x-hostable-operator-token: $TOKEN" \
  > "$WORK/start.json"
grep -q '"action":"start"' "$WORK/start.json"

curl -fsS "$BASE/api/servers/smoke-mc/status" \
  -H "x-hostable-operator-token: $TOKEN" \
  > "$WORK/status-active.json"
grep -q '"active":"active"' "$WORK/status-active.json"

curl -fsS -X POST "$BASE/api/servers/smoke-mc/restart" \
  -H "x-hostable-operator-token: $TOKEN" \
  > "$WORK/restart.json"
grep -q '"action":"restart"' "$WORK/restart.json"

curl -fsS -X POST "$BASE/api/servers/smoke-mc/stop" \
  -H "x-hostable-operator-token: $TOKEN" \
  > "$WORK/stop.json"
grep -q '"action":"stop"' "$WORK/stop.json"
grep -q 'restart hostable-smoke-mc.service' "$HOSTABLE_FAKE_SYSTEMCTL_LOG"

curl -fsS "$BASE/api/servers/smoke-mc/logs?lines=25" \
  -H "x-hostable-operator-token: $TOKEN" \
  > "$WORK/logs.json"
grep -q '"service":"hostable-smoke-mc.service"' "$WORK/logs.json"
grep -q 'Hostable smoke log line' "$WORK/logs.json"

curl -fsS "$BASE/api/servers/smoke-mc/config" \
  -H "x-hostable-operator-token: $TOKEN" \
  > "$WORK/config-get.json"
grep -q 'server.properties' "$WORK/config-get.json"

curl -fsS -X PUT "$BASE/api/servers/smoke-mc/config" \
  -H "x-hostable-operator-token: $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"files":[{"path":"server.properties","format":"properties","contents":"motd=new\n"}]}' \
  > "$WORK/config-put.json"
test "$(cat "$WORK/server/server.properties")" = 'motd=new'

curl -fsS -X POST "$BASE/api/servers/smoke-mc/backups/run" \
  -H "x-hostable-operator-token: $TOKEN" \
  > "$WORK/backup-first.json"
grep -q '"server_id":"smoke-mc"' "$WORK/backup-first.json"
test -s "$(find "$WORK/backups" -name '*.tar.zst' | head -n 1)"

curl -fsS -X PUT "$BASE/api/settings/backups" \
  -H "x-hostable-operator-token: $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"local_keep_last":1,"s3_keep_last":1}' \
  > "$WORK/backup-settings.json"

sleep 1
curl -fsS -X POST "$BASE/api/servers/smoke-mc/backups/run" \
  -H "x-hostable-operator-token: $TOKEN" \
  > "$WORK/backup-second.json"
grep -q '"pruned_local_paths":\[' "$WORK/backup-second.json"
archive_count="$(find "$WORK/backups" -name '*.tar.zst' | wc -l | tr -d ' ')"
manifest_count="$(find "$WORK/backups" -name '*.manifest.json' | wc -l | tr -d ' ')"
test "$archive_count" = 1
test "$manifest_count" = 1

echo "daemon-smoke-ok"
