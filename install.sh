#!/usr/bin/env sh
set -eu

REPO="${HOSTABLE_REPO:-blankeos/hostable}"
VERSION="${HOSTABLE_VERSION:-latest}"
INSTALL_DIR="${HOSTABLE_INSTALL_DIR:-/usr/local/bin}"
STATE_DIR="${HOSTABLE_STATE_DIR:-/var/lib/hostable}"
CONFIG_DIR="${HOSTABLE_CONFIG_DIR:-/etc/hostable}"
SERVER_DIR="${HOSTABLE_SERVER_DIR:-/srv/hostable}"
SERVICE_FILE="/etc/systemd/system/hostabled.service"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing required command: $1" >&2
    exit 1
  }
}

as_root() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  else
    sudo "$@"
  fi
}

need curl
need tar
need uname

if ! command -v systemctl >/dev/null 2>&1; then
  echo "Hostable requires systemd for the MVP installer." >&2
  exit 1
fi

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

if [ "$VERSION" = "latest" ]; then
  RELEASE_URL="https://api.github.com/repos/$REPO/releases/latest"
else
  RELEASE_URL="https://api.github.com/repos/$REPO/releases/tags/$VERSION"
fi

echo "Installing Hostable for $TARGET from $REPO..."
ASSET_URL="$(curl -fsSL "$RELEASE_URL" \
  | sed -n 's/.*"browser_download_url": "\([^"]*'"$TARGET"'[^"]*\.tar\.[xz][xz]*\)".*/\1/p' \
  | head -n 1)"

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

if [ -z "$HOSTABLE_BIN" ] || [ -z "$HOSTABLED_BIN" ]; then
  echo "Release archive did not contain hostable and hostabled binaries." >&2
  exit 1
fi

as_root install -d "$INSTALL_DIR" "$STATE_DIR" "$STATE_DIR/web-admin" "$CONFIG_DIR" "$SERVER_DIR/servers" "$SERVER_DIR/backups"
as_root install -m 0755 "$HOSTABLE_BIN" "$INSTALL_DIR/hostable"
as_root install -m 0755 "$HOSTABLED_BIN" "$INSTALL_DIR/hostabled"

if ! id hostable >/dev/null 2>&1; then
  as_root useradd --system --create-home --shell /usr/sbin/nologin hostable
fi

as_root chown -R hostable:hostable "$SERVER_DIR" "$STATE_DIR"

cat > "$TMP_DIR/hostabled.service" <<EOF
[Unit]
Description=Hostable daemon
After=network.target

[Service]
ExecStart=$INSTALL_DIR/hostabled --bind 127.0.0.1:8080 --registry $STATE_DIR/registry.sqlite --backup-root $SERVER_DIR/backups --admin-dist $STATE_DIR/web-admin
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

as_root install -m 0644 "$TMP_DIR/hostabled.service" "$SERVICE_FILE"
as_root systemctl daemon-reload
as_root systemctl enable hostabled
as_root systemctl restart hostabled

echo "Hostable installed."
echo "Run: hostable"
