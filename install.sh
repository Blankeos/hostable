#!/usr/bin/env sh
set -eu

REPO="${HOSTABLE_REPO:-blankeos/hostable}"
VERSION="${HOSTABLE_VERSION:-latest}"
DOWNLOAD_URL="${HOSTABLE_DOWNLOAD_URL:-}"
INSTALL_DIR="${HOSTABLE_INSTALL_DIR:-/usr/local/bin}"
STATE_DIR="${HOSTABLE_STATE_DIR:-/var/lib/hostable}"
CONFIG_DIR="${HOSTABLE_CONFIG_DIR:-/etc/hostable}"
SERVER_DIR="${HOSTABLE_SERVER_DIR:-/srv/hostable}"
NO_SUDO="${HOSTABLE_NO_SUDO:-0}"
SKIP_SERVICE="${HOSTABLE_SKIP_SERVICE:-0}"
SERVICE_FILE="/etc/systemd/system/hostabled.service"
OPERATOR_TOKEN="$CONFIG_DIR/operator-token"
SECRETS_FILE="$CONFIG_DIR/secrets.env"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing required command: $1" >&2
    exit 1
  }
}

as_root() {
  if [ "$(id -u)" -eq 0 ] || [ "$NO_SUDO" = "1" ]; then
    "$@"
  else
    sudo "$@"
  fi
}

need curl
need uname

if [ "$SKIP_SERVICE" != "1" ] && ! command -v systemctl >/dev/null 2>&1; then
  echo "Hostable requires systemd for the MVP installer." >&2
  exit 1
fi

if [ -r /etc/os-release ]; then
  . /etc/os-release
  case "${ID:-}" in
    debian|ubuntu) ;;
    *)
      case " ${ID_LIKE:-} " in
        *" debian "*) ;;
        *) echo "Hostable supports Debian and Ubuntu as best effort. Detected: ${ID:-unknown}" >&2; exit 1 ;;
      esac
      ;;
  esac
fi

if command -v apt-get >/dev/null 2>&1; then
  as_root apt-get update
  as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y ca-certificates tar gzip xz-utils coreutils findutils openssl ufw
fi

need tar

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux) OS_PART="unknown-linux-gnu" ;;
  *) echo "Unsupported OS: $OS" >&2; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64) TARGET="x86_64-$OS_PART" ;;
  aarch64|arm64) TARGET="aarch64-$OS_PART" ;;
  *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

if [ -n "$DOWNLOAD_URL" ]; then
  echo "Installing Hostable for $TARGET from $DOWNLOAD_URL..."
  ASSET_URL="$DOWNLOAD_URL"
else
  if [ "$VERSION" = "latest" ]; then
    RELEASE_URL="https://api.github.com/repos/$REPO/releases/latest"
  else
    RELEASE_URL="https://api.github.com/repos/$REPO/releases/tags/$VERSION"
  fi

  echo "Installing Hostable for $TARGET from $REPO..."
  ASSET_URL="$(curl -fsSL "$RELEASE_URL" \
    | sed -n 's#.*"browser_download_url"[ ]*:[ ]*"\(https://[^"]*/hostable-'"$TARGET"'\.tar\.xz\)".*#\1#p' \
    | head -n 1)"
fi

if [ -z "$ASSET_URL" ]; then
  echo "Could not find a release archive for $TARGET." >&2
  exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

curl -fL "$ASSET_URL" -o "$TMP_DIR/hostable.tar"
tar -xf "$TMP_DIR/hostable.tar" -C "$TMP_DIR"

HOSTABLE_BIN="$(find "$TMP_DIR" -type f -name hostable | head -n 1)"
HOSTABLED_BIN="$(find "$TMP_DIR" -type f -name hostabled | head -n 1)"
ADMIN_INDEX="$(find "$TMP_DIR" -type f -path '*/web-admin/index.html' | head -n 1)"

if [ -z "$HOSTABLE_BIN" ] || [ -z "$HOSTABLED_BIN" ]; then
  echo "Release archive did not contain hostable and hostabled binaries." >&2
  exit 1
fi

as_root install -d "$INSTALL_DIR" "$STATE_DIR" "$STATE_DIR/web-admin" "$CONFIG_DIR" "$SERVER_DIR/servers" "$SERVER_DIR/backups"
as_root install -m 0755 "$HOSTABLE_BIN" "$INSTALL_DIR/hostable"
as_root install -m 0755 "$HOSTABLED_BIN" "$INSTALL_DIR/hostabled"

if [ -n "$ADMIN_INDEX" ]; then
  ADMIN_DIR="$(dirname "$ADMIN_INDEX")"
  as_root cp -R "$ADMIN_DIR/." "$STATE_DIR/web-admin/"
fi

if [ ! -f "$OPERATOR_TOKEN" ]; then
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32 > "$TMP_DIR/operator-token"
  else
    dd if=/dev/urandom bs=32 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n' > "$TMP_DIR/operator-token"
  fi
  as_root install -m 0600 -o root -g root "$TMP_DIR/operator-token" "$OPERATOR_TOKEN"
fi

if [ ! -f "$SECRETS_FILE" ]; then
  cat > "$TMP_DIR/secrets.env" <<'EOF'
# Optional S3 credentials for Hostable backups.
# AWS_ACCESS_KEY_ID=
# AWS_SECRET_ACCESS_KEY=
EOF
  as_root install -m 0600 "$TMP_DIR/secrets.env" "$SECRETS_FILE"
fi

if [ "$SKIP_SERVICE" != "1" ]; then
  if ! id hostable >/dev/null 2>&1; then
    as_root useradd --system --create-home --shell /usr/sbin/nologin hostable
  fi

  # Game servers (and anything they run) own only the server data. The registry,
  # web admin assets and config stay root-owned so a compromised game server
  # cannot steer the root daemon.
  as_root chown -R hostable:hostable "$SERVER_DIR"
  as_root chown -R root:root "$STATE_DIR" "$CONFIG_DIR"
  as_root chmod 0755 "$STATE_DIR" "$CONFIG_DIR"
  as_root chmod 0600 "$OPERATOR_TOKEN" "$SECRETS_FILE"

  cat > "$TMP_DIR/hostabled.service" <<EOF
[Unit]
Description=Hostable daemon
After=network.target

[Service]
EnvironmentFile=-$SECRETS_FILE
ExecStart=$INSTALL_DIR/hostabled --bind 0.0.0.0:8080 --registry $STATE_DIR/registry.sqlite --backup-root $SERVER_DIR/backups --settings $CONFIG_DIR/settings.json --admin-dist $STATE_DIR/web-admin --operator-token $OPERATOR_TOKEN
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

  as_root install -m 0644 "$TMP_DIR/hostabled.service" "$SERVICE_FILE"
  as_root systemctl daemon-reload
  as_root systemctl enable hostabled
  # Older installs opened the daemon port to the world. Close it unless the web admin is on.
  if ! grep -Eq '"enabled"[[:space:]]*:[[:space:]]*true' "$CONFIG_DIR/settings.json" 2>/dev/null; then
    as_root ufw delete allow 8080/tcp >/dev/null 2>&1 || true
  fi
  as_root systemctl restart hostabled
else
  echo "Skipping systemd service setup."
fi

echo "Hostable installed."
echo "Run: sudo hostable"
echo "The daemon only accepts local connections until you enable the web admin in Settings."
